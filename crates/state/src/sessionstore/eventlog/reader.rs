//! Record framing over the raw log bytes (session_replay_reader.go).
//!
//! The reader walks newline-free JSON objects: one record per object,
//! whitespace between records is skipped, a bare null word is handed
//! back verbatim so the zero-record rule in Go applies, and every byte
//! read is charged against the encoded-bytes budget.

use std::io::{self, BufRead, BufReader, Read};

use super::{
    SESSION_REPLAY_MAX_PAYLOAD_BYTES, SessionReplayLimitError, session_replay_limit_error,
};

/// Outcome of a framing step: clean EOF, budget refusal, or damage
/// (torn input, undecodable bytes).
#[derive(Debug)]
pub(crate) enum NextError {
    /// The log ended cleanly between records.
    Eof,
    /// A byte or record budget tripped; the log was left untouched.
    Limit(SessionReplayLimitError),
    /// Torn or undecodable input.
    Damaged(io::Error),
}

/// Frames one record at a time (Go: sessionRecordReader).
pub(crate) struct SessionRecordReader<R> {
    reader: BufReader<R>,
    path: String,
    max_bytes: i64,
    /// Cap on a single record's encoded size (Go: the
    /// sessionReplayMaxPayloadBytes const, kept a field for tests).
    pub(crate) max_payload_bytes: i64,
    offset: i64,
    buffer: Vec<u8>,
}

impl<R: Read> SessionRecordReader<R> {
    pub(crate) fn new(reader: R, path: &str, max_bytes: i64) -> Self {
        Self {
            reader: BufReader::new(reader),
            path: path.to_owned(),
            max_bytes,
            max_payload_bytes: SESSION_REPLAY_MAX_PAYLOAD_BYTES,
            offset: 0,
            buffer: Vec::new(),
        }
    }

    /// Bytes consumed so far (Go: the offset field).
    pub(crate) fn offset(&self) -> i64 {
        self.offset
    }

    fn read_byte(&mut self) -> Result<u8, NextError> {
        let mut one = [0u8; 1];
        loop {
            match self.reader.read(&mut one) {
                Ok(0) => return Err(NextError::Eof),
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(NextError::Damaged(e)),
            }
        }
        self.offset += 1;
        if self.offset > self.max_bytes {
            return Err(NextError::Limit(session_replay_limit_error(
                &self.path,
                "encoded_bytes",
                self.offset,
                self.max_bytes,
            )));
        }
        Ok(one[0])
    }

    /// Frames the next record, returning its raw bytes and the offset
    /// it started at (Go: sessionRecordReader.next).
    pub(crate) fn next(&mut self) -> Result<(Vec<u8>, i64), NextError> {
        let head = loop {
            let byte = self.read_byte()?;
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' => continue,
                other => break other,
            }
        };
        self.buffer.clear();
        let start = self.offset - 1;
        self.buffer.push(head);
        if head == b'n' {
            // The null word: four bytes are consumed and returned
            // verbatim so json.Unmarshal sees "null" (Go reads the
            // remaining three, converting EOF to UnexpectedEOF).
            for _ in 0..3 {
                let byte = self.read_byte().map_err(|err| match err {
                    NextError::Eof => NextError::Damaged(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "unexpected EOF",
                    )),
                    other => other,
                })?;
                self.buffer.push(byte);
            }
            return Ok((std::mem::take(&mut self.buffer), start));
        }
        if head != b'{' {
            return Err(NextError::Damaged(io::Error::other(
                "session event record must be an object",
            )));
        }
        let mut scan = RecordScan {
            depth: 1,
            quoted: false,
            escaped: false,
        };
        while scan.depth != 0 {
            self.advance(&mut scan)?;
        }
        Ok((std::mem::take(&mut self.buffer), start))
    }

    /// Consumes at least one more byte of the current record, charging
    /// every byte to the budgets (Go: sessionRecordReader.advance).
    fn advance(&mut self, scan: &mut RecordScan) -> Result<(), NextError> {
        let room = {
            let chunk = self.reader.fill_buf().map_err(NextError::Damaged)?;
            if chunk.is_empty() {
                return Err(NextError::Damaged(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "unexpected EOF",
                )));
            }
            (chunk.len() as i64)
                .min(self.max_bytes.saturating_sub(self.offset))
                .min(
                    self.max_payload_bytes
                        .saturating_sub(self.buffer.len() as i64),
                )
        };
        if room <= 0 {
            // A budget is exhausted mid-record: route one byte through
            // the byte counter so the matching refusal fires (Go reads
            // a byte, then refuses on the record cap).
            let byte = self.read_byte()?;
            if self.buffer.len() as i64 >= self.max_payload_bytes {
                return Err(NextError::Limit(session_replay_limit_error(
                    &self.path,
                    "event_record_bytes",
                    self.buffer.len() as i64 + 1,
                    self.max_payload_bytes,
                )));
            }
            self.buffer.push(byte);
            scan.feed(std::slice::from_ref(&byte));
            return Ok(());
        }
        let fed = {
            let chunk = self.reader.fill_buf().map_err(NextError::Damaged)?;
            let fed = scan.feed(&chunk[..room as usize]);
            self.buffer.extend_from_slice(&chunk[..fed]);
            fed
        };
        self.reader.consume(fed);
        self.offset += fed as i64;
        Ok(())
    }
}

/// Brace-depth scanner that stays inside string literals (Go:
/// recordScan).
struct RecordScan {
    depth: i64,
    quoted: bool,
    escaped: bool,
}

impl RecordScan {
    fn feed(&mut self, p: &[u8]) -> usize {
        for (i, &b) in p.iter().enumerate() {
            if self.quoted {
                if self.escaped {
                    self.escaped = false;
                } else if b == b'\\' {
                    self.escaped = true;
                } else if b == b'"' {
                    self.quoted = false;
                }
                continue;
            }
            match b {
                b'"' => self.quoted = true,
                b'{' | b'[' => self.depth += 1,
                b'}' | b']' => {
                    self.depth -= 1;
                    if self.depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
        }
        p.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(body: &[u8]) -> SessionRecordReader<&[u8]> {
        SessionRecordReader::new(body, "log", i64::MAX)
    }

    #[test]
    fn frames_records_and_offsets() {
        let body = b"{\"a\":1} {\"b\":[1,2]}";
        let mut r = reader(body);
        let (raw, start) = r.next().unwrap();
        assert_eq!(raw, b"{\"a\":1}");
        assert_eq!(start, 0);
        let (raw, start) = r.next().unwrap();
        assert_eq!(raw, b"{\"b\":[1,2]}");
        assert_eq!(start, 8);
        assert!(matches!(r.next(), Err(NextError::Eof)));
        assert_eq!(r.offset(), 19);
    }

    #[test]
    fn skips_newlines_and_returns_null_word() {
        let mut r = reader(b"\nnull\n{\"a\":1}");
        let (raw, start) = r.next().unwrap();
        assert_eq!(raw, b"null");
        assert_eq!(start, 1);
        let (raw, _) = r.next().unwrap();
        assert_eq!(raw, b"{\"a\":1}");
    }

    #[test]
    fn torn_null_word_is_damaged() {
        let mut r = reader(b"nul");
        match r.next() {
            Err(NextError::Damaged(e)) => {
                assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn non_object_is_damaged() {
        let mut r = reader(b"garbage");
        match r.next() {
            Err(NextError::Damaged(e)) => {
                assert_eq!(e.to_string(), "session event record must be an object");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn torn_record_is_damaged() {
        let mut r = reader(b"{\"a\":");
        match r.next() {
            Err(NextError::Damaged(e)) => {
                assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn braces_inside_strings_do_not_count() {
        let mut r = reader(b"{\"a\":\"}{\\\"\"}");
        let (raw, _) = r.next().unwrap();
        assert_eq!(raw, b"{\"a\":\"}{\\\"\"}");
        assert!(matches!(r.next(), Err(NextError::Eof)));
    }

    #[test]
    fn encoded_bytes_budget_fires() {
        let mut r = SessionRecordReader::new(&b"  {\"a\":1}"[..], "log", 3);
        match r.next() {
            Err(NextError::Limit(err)) => {
                assert_eq!(err.resource, "encoded_bytes");
                assert_eq!(err.value, 4);
                assert_eq!(err.limit, 3);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn record_payload_budget_fires() {
        let mut r = SessionRecordReader::new(&b"{\"a\":1}"[..], "log", i64::MAX);
        r.max_payload_bytes = 4;
        match r.next() {
            Err(NextError::Limit(err)) => {
                assert_eq!(err.resource, "event_record_bytes");
                assert_eq!(err.value, 5);
                assert_eq!(err.limit, 4);
            }
            other => panic!("{other:?}"),
        }
    }

    /// The reference outcome of one framing step, independent of the
    /// implementation types.
    #[derive(Debug, PartialEq)]
    enum RefErr {
        Eof,
        Damaged(String),
        Limit {
            resource: String,
            value: i64,
            limit: i64,
        },
    }

    fn map_ref(err: NextError) -> RefErr {
        match err {
            NextError::Eof => RefErr::Eof,
            NextError::Damaged(err) => RefErr::Damaged(match err.kind() {
                io::ErrorKind::UnexpectedEof => "unexpected EOF".to_string(),
                _ => err.to_string(),
            }),
            NextError::Limit(err) => RefErr::Limit {
                resource: err.resource.to_string(),
                value: err.value,
                limit: err.limit,
            },
        }
    }

    /// Converts a clean EOF seen mid-record into the torn-tail damage the
    /// reference reports for the same bytes.
    fn to_unexpected_eof(err: NextError) -> RefErr {
        match err {
            NextError::Eof => RefErr::Damaged("unexpected EOF".to_string()),
            other => map_ref(other),
        }
    }

    /// Frames one record byte at a time, the way the reference scan does.
    fn reference_next(r: &mut SessionRecordReader<&[u8]>) -> Result<(Vec<u8>, i64), RefErr> {
        r.buffer.clear();
        let head = loop {
            let byte = r.read_byte().map_err(map_ref)?;
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' => continue,
                other => break other,
            }
        };
        let start = r.offset - 1;
        r.buffer.push(head);
        if head == b'n' {
            for _ in 0..3 {
                // Mid-record EOF is a torn tail, not a clean end.
                let byte = r.read_byte().map_err(to_unexpected_eof)?;
                r.buffer.push(byte);
            }
            return Ok((std::mem::take(&mut r.buffer), start));
        }
        if head != b'{' {
            return Err(RefErr::Damaged(
                "session event record must be an object".to_string(),
            ));
        }
        let mut depth: i64 = 1;
        let (mut quoted, mut escaped) = (false, false);
        while depth != 0 {
            let byte = r.read_byte().map_err(to_unexpected_eof)?;
            r.buffer.push(byte);
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
                continue;
            }
            match byte {
                b'"' => quoted = true,
                b'{' | b'[' => depth += 1,
                b'}' | b']' => depth -= 1,
                _ => {}
            }
        }
        Ok((std::mem::take(&mut r.buffer), start))
    }

    #[test]
    fn bulk_framing_agrees_with_byte_at_a_time() {
        // Every record shape the log can hold, adjacent and pretty-printed,
        // must frame identically through the buffered scan and the slow path.
        let atoms: [&str; 14] = [
            "{}",
            r#"{"a":1}"#,
            r#"{"s":"x{y}[z]"}"#,
            r#"{"e":"q\"uote\\"}"#,
            r#"{"n":{"m":[1,{"k":"v"}]}}"#,
            "\n",
            "  ",
            "\r\n\t",
            "null",
            r#"{"u":"\u00e9 é 中文"}"#,
            r#"{"unterminated":"#,
            r#"{"deep":[[[[{"x":[]}]]]]}"#,
            "x",
            r#"{"schema_version":1,"type":"replace","messages":[]}"#,
        ];
        let mut state: u64 = 7;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize
        };
        for iter in 0..200 {
            let mut doc = String::new();
            for _ in 0..1 + next() % 6 {
                doc.push_str(atoms[next() % atoms.len()]);
            }
            let data = doc.clone().into_bytes();
            let max_bytes = 1 + (next() % (data.len() + 40)) as i64;
            let mut bulk = SessionRecordReader::new(&data[..], "p", max_bytes);
            let mut reference = SessionRecordReader::new(&data[..], "p", max_bytes);
            for call in 0..8 {
                let got = bulk.next().map_err(map_ref);
                let want = reference_next(&mut reference);
                assert_eq!(got, want, "iter {iter} call {call} on {doc:?}");
                assert_eq!(
                    bulk.offset(),
                    reference.offset(),
                    "iter {iter} call {call} on {doc:?}"
                );
                if got.is_err() {
                    break;
                }
            }
        }
    }
}
