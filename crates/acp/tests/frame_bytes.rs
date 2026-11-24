//! ACP-001: NDJSON frame codec, byte level.
//!
//! Assertions run on real bytes: separators, blank lines, CRLF, whitespace,
//! the partial trailing line, and the 32 MiB boundary. The over-limit path is
//! a typed end reason, never a fabricated `-32603` response frame.

use reasonix_acp::frame::{FrameTooLarge, MAX_MESSAGE_BYTES, encode_frame, next_frame};

#[test]
fn empty_buffer_has_no_frame() {
    assert!(next_frame(b"").unwrap().is_none());
}

#[test]
fn blank_line_consumed_but_skipped() {
    let frame = next_frame(b"\n").unwrap().unwrap();
    assert!(frame.is_empty());
    assert_eq!(frame.consumed, 1);
    assert!(!frame.partial);
}

#[test]
fn crlf_line_trims_both_endings_and_surrounding_ws() {
    let raw = b"  \t{\"jsonrpc\":\"2.0\"}\r\n";
    let frame = next_frame(raw).unwrap().unwrap();
    assert_eq!(frame.line, br#"{"jsonrpc":"2.0"}"#);
    assert_eq!(frame.consumed, raw.len());
    assert!(!frame.partial);
}

#[test]
fn lf_only_and_trailing_partial_line_are_detected() {
    let lf = next_frame(br#"{"a":1}"#).unwrap().unwrap();
    assert!(lf.partial);
    assert_eq!(lf.consumed, lf.line.len());

    let crlf = next_frame(b"x\r\r\n").unwrap().unwrap();
    assert_eq!(crlf.line, b"x");
}

#[test]
fn two_frames_split_on_first_newline() {
    let buf = br#"{"a":1}"#.to_vec();
    let mut buf = [buf, b"\n".to_vec(), br#"{"b":2}"#.to_vec(), b"\n".to_vec()].concat();
    let first = next_frame(&buf).unwrap().unwrap();
    assert_eq!(first.line, br#"{"a":1}"#);
    buf.drain(..first.consumed);
    let second = next_frame(&buf).unwrap().unwrap();
    assert_eq!(second.line, br#"{"b":2}"#);
}

#[test]
fn line_at_exact_budget_passes_one_over_fails_typed() {
    let ok = vec![b'x'; MAX_MESSAGE_BYTES];
    let frame = next_frame(&ok).unwrap();
    assert!(frame.is_some());

    let mut over = vec![b'x'; MAX_MESSAGE_BYTES + 1];
    over.push(b'\n');
    let err = next_frame(&over).unwrap_err();
    assert_eq!(
        err,
        FrameTooLarge {
            size: MAX_MESSAGE_BYTES + 2,
            limit: MAX_MESSAGE_BYTES,
        }
    );
    // The budget is a byte count, not a u32-truncated length.
    assert_eq!(MAX_MESSAGE_BYTES, 33_554_432);
}

#[test]
fn outbound_frame_appends_single_lf_and_keeps_html_chars_raw() {
    let body = serde_json::json!({"raw": "<a>&\"b\""});
    let frame = encode_frame(&body);
    assert_eq!(frame.last(), Some(&b'\n'));
    assert!(!frame.windows(2).any(|w| w == b"\n\n"));
    let text = String::from_utf8(frame).unwrap();
    assert!(text.contains("<a>&"));
    assert!(text.ends_with("}\n"));
}
