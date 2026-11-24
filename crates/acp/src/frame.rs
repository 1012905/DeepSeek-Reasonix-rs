//! NDJSON framing for the ACP byte channel (server.go:16,136-200).
//!
//! The layer works on plain byte slices: no stdin/stdout, no runtime. One
//! JSON-RPC frame rides one line; the read side splits on `\n`, trims trailing
//! CR/LF and surrounding ASCII whitespace, and enforces the 32 MiB per-line
//! budget. The write side serializes a JSON value and appends exactly one
//! `\n`, matching `json.Encoder.Encode` with HTML escaping disabled.

use serde::Serialize;

/// Inbound per-line budget: 32 MiB (`32 << 20`, server.go:16). Counted in wire
/// bytes, so a payload near the limit is sized by length, never truncated with
/// a `u32` cast.
pub const MAX_MESSAGE_BYTES: usize = 32 << 20;

/// A line that crossed [`MAX_MESSAGE_BYTES`]. The framing loop stops on this;
/// the reference does not write an error frame for it (server.go:173-175).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("acp: message exceeds size limit")]
pub struct FrameTooLarge {
    /// Wire bytes observed for the line, including the separator when present.
    pub size: usize,
    /// Configured budget this line crossed.
    pub limit: usize,
}

/// One NDJSON line pulled from the input buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    /// Trimmed line bytes; empty when the source line was blank or whitespace.
    pub line: &'a [u8],
    /// Bytes consumed from the input, including the line separator.
    pub consumed: usize,
    /// True when the line ended at buffer end without a trailing newline, the
    /// partial final frame the reference still processes on EOF.
    pub partial: bool,
}

impl<'a> Frame<'a> {
    /// True when the trimmed line carries no payload; the read loop skips these
    /// (server.go:144).
    pub fn is_empty(&self) -> bool {
        self.line.is_empty()
    }
}

fn is_ascii_space(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\n' || c == b'\r'
}

fn trim_ascii_ws(b: &[u8]) -> &[u8] {
    let mut i = 0;
    let mut j = b.len();
    while i < j && is_ascii_space(b[i]) {
        i += 1;
    }
    while j > i && is_ascii_space(b[j - 1]) {
        j -= 1;
    }
    &b[i..j]
}

/// Reads the next frame from `buf`.
///
/// Returns `Ok(None)` when the buffer holds no bytes. A missing newline marks
/// the trailing partial line, matching the EOF behavior of `readLine`. A line
/// over the budget returns [`FrameTooLarge`] before any JSON parsing happens.
pub fn next_frame(buf: &[u8]) -> Result<Option<Frame<'_>>, FrameTooLarge> {
    if buf.is_empty() {
        return Ok(None);
    }

    let newline = buf.iter().position(|&c| c == b'\n');
    let raw_len = newline.unwrap_or(buf.len());
    let consumed = newline.map_or(buf.len(), |idx| idx + 1);
    let partial = newline.is_none();

    // The reference counts the accumulated buffer, separator included, against
    // the budget (server.go:173).
    let size = raw_len + usize::from(newline.is_some());
    if size > MAX_MESSAGE_BYTES {
        return Err(FrameTooLarge {
            size,
            limit: MAX_MESSAGE_BYTES,
        });
    }

    let mut end = raw_len;
    while end > 0 && (buf[end - 1] == b'\n' || buf[end - 1] == b'\r') {
        end -= 1;
    }
    let line = trim_ascii_ws(&buf[..end]);
    Ok(Some(Frame {
        line,
        consumed,
        partial,
    }))
}

/// Serializes one outbound frame and appends the NDJSON newline.
///
/// `serde_json` leaves `<`, `>`, `&` unescaped by default, matching the
/// reference encoder's `SetEscapeHTML(false)` (server.go:108-112).
pub fn encode_frame<T: Serialize>(value: &T) -> Vec<u8> {
    let mut out = serde_json::to_vec(value).expect("ACP outbound payloads serialize");
    out.push(b'\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer_yields_no_frame() {
        assert!(next_frame(b"").unwrap().is_none());
    }

    #[test]
    fn blank_line_is_skipped() {
        let frame = next_frame(b"\n").unwrap().unwrap();
        assert!(frame.is_empty());
        assert_eq!(frame.consumed, 1);
        assert!(!frame.partial);
    }

    #[test]
    fn crlf_and_surrounding_ws_are_trimmed() {
        let frame = next_frame(b"  \t{\"jsonrpc\":\"2.0\"}\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(frame.line, br#"{"jsonrpc":"2.0"}"#);
        assert_eq!(frame.consumed, b"  \t{\"jsonrpc\":\"2.0\"}\r\n".len());
    }

    #[test]
    fn trailing_partial_line_without_newline_is_returned() {
        let frame = next_frame(br#"{"jsonrpc":"2.0"}"#).unwrap().unwrap();
        assert!(frame.partial);
        assert_eq!(frame.line, br#"{"jsonrpc":"2.0"}"#);
        assert_eq!(frame.consumed, frame.line.len());
    }

    #[test]
    fn second_frame_starts_after_first_newline() {
        let buf = b"{\"a\":1}\n{\"b\":2}\n";
        let first = next_frame(buf).unwrap().unwrap();
        assert_eq!(first.line, br#"{"a":1}"#);
        let second = next_frame(&buf[first.consumed..]).unwrap().unwrap();
        assert_eq!(second.line, br#"{"b":2}"#);
    }

    #[test]
    fn line_at_exact_budget_passes_one_byte_over_fails() {
        let ok = vec![b'x'; MAX_MESSAGE_BYTES];
        assert!(next_frame(&ok).is_ok());

        let mut over = vec![b'x'; MAX_MESSAGE_BYTES + 1];
        over.push(b'\n');
        let err = next_frame(&over).unwrap_err();
        assert_eq!(err.limit, MAX_MESSAGE_BYTES);
        assert_eq!(err.size, MAX_MESSAGE_BYTES + 2);
    }

    #[test]
    fn outbound_frame_ends_with_single_newline() {
        let body = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"ping"});
        let frame = encode_frame(&body);
        assert_eq!(frame.last(), Some(&b'\n'));
        assert_eq!(&frame[frame.len() - 2..], b"}\n");
        assert!(!frame.windows(2).any(|w| w == b"\n\n"));
    }
}
