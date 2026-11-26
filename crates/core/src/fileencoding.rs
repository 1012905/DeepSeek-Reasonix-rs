//! Byte-level detection, decoding, and encoding for user-authored text files.
//!
//! The BOM, BOM-less UTF-16, strict UTF-8, GB18030, GBK, and lossy cascade
//! keeps CJK Windows files readable without silently changing their bytes.

use std::borrow::Cow;
use std::fs;
use std::io;
use std::path::Path;

use encoding_rs::{CoderResult, Encoding};

/// A detected file encoding. The discriminants match the Go Kind iota
/// order because checkpoints persist the kind by value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// Plain UTF-8 without a BOM.
    Utf8 = 0,
    /// UTF-8 with a leading BOM (EF BB BF).
    Utf8Bom = 1,
    /// UTF-16 little-endian with a BOM (FF FE).
    Utf16Le = 2,
    /// UTF-16 big-endian with a BOM (FE FF).
    Utf16Be = 3,
    /// The Chinese national charset, a superset of GBK.
    Gb18030 = 4,
    /// Not valid UTF-8 and no charset restores it byte for byte; bytes
    /// pass through untouched so a rewrite keeps what it did not edit.
    LossyUtf8 = 5,
    /// BOM-less UTF-16 little-endian, detected from the NUL pattern.
    Utf16LeNoBom = 6,
    /// BOM-less UTF-16 big-endian.
    Utf16BeNoBom = 7,
    /// CP936, where GB18030 cannot restore the bytes, such as its 0x80 euro.
    Gbk = 8,
}

fn charsets() -> [(Kind, &'static Encoding); 2] {
    [
        (Kind::Gb18030, encoding_rs::GB18030),
        (Kind::Gbk, encoding_rs::GBK),
    ]
}

/// Mirrors the Go runeLen helper: the encoded length for a lead byte,
/// or 1 for bytes that cannot start a rune.
fn rune_len(b: u8) -> usize {
    if b & 0x80 == 0 {
        1
    } else if b & 0xE0 == 0xC0 {
        2
    } else if b & 0xF0 == 0xE0 {
        3
    } else if b & 0xF8 == 0xF0 {
        4
    } else {
        1
    }
}

/// Mirrors Go utf8.DecodeRune: Some((char, len)) for the rune at the
/// head of b, None when Go reports (RuneError, 1) — invalid, truncated
/// or overlong sequences all collapse to that single result.
fn decode_rune(b: &[u8]) -> Option<(char, usize)> {
    let first = b[0];
    if first & 0xC0 == 0x80 || first >= 0xF8 {
        return None;
    }
    let len = rune_len(first);
    if len == 1 {
        return Some((char::from(first), 1));
    }
    if b.len() < len {
        return None;
    }
    let mut value = match len {
        2 => u32::from(first & 0x1F),
        3 => u32::from(first & 0x0F),
        _ => u32::from(first & 0x07),
    };
    for &continuation in &b[1..len] {
        if continuation & 0xC0 != 0x80 {
            return None;
        }
        value = (value << 6) | u32::from(continuation & 0x3F);
    }
    let min = match len {
        2 => 0x80,
        3 => 0x800,
        _ => 0x10000,
    };
    if value < min || (0xD800..=0xDFFF).contains(&value) || value > 0x10FFFF {
        return None;
    }
    char::from_u32(value).map(|c| (c, len))
}

/// Trims a trailing partial UTF-8 rune so a fragment boundary falls on a
/// character edge. Walks back at most three bytes; the tail is dropped
/// only when the rune is both invalid here and longer than the bytes
/// that remain, so merely-invalid input stays untouched.
pub fn trim_partial_rune(peek: &[u8]) -> &[u8] {
    let n = peek.len();
    if n == 0 {
        return peek;
    }
    let mut i = n - 1;
    while n - i < 4 {
        let b = peek[i];
        if b & 0xC0 != 0x80 {
            let rem = &peek[i..];
            let invalid = decode_rune(rem).is_none();
            if invalid && rune_len(b) > rem.len() {
                return &peek[..i];
            }
            return peek;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    peek
}

/// Heuristic for BOM-less UTF-16: ASCII-heavy text has a NUL beside
/// every character. Detects LE (NULs on odd offsets) or BE (even
/// offsets); CJK text has no NUL bytes and does not trigger either way.
pub fn detect_utf16_no_bom(b: &[u8]) -> Option<Kind> {
    if b.len() < 16 {
        return None;
    }
    let n = b.len() & !1;
    let mut even_nul = 0usize;
    let mut odd_nul = 0usize;
    for (i, &byte) in b[..n].iter().enumerate() {
        if byte == 0 {
            if i % 2 == 0 {
                even_nul += 1;
            } else {
                odd_nul += 1;
            }
        }
    }
    let half = n / 2;
    if odd_nul * 10 >= half * 3 && even_nul * 20 <= half {
        return Some(Kind::Utf16LeNoBom);
    }
    if even_nul * 10 >= half * 3 && odd_nul * 20 <= half {
        return Some(Kind::Utf16BeNoBom);
    }
    None
}

/// Decodes UTF-16 bytes to UTF-8. An odd trailing byte is dropped; lone
/// surrogates become U+FFFD, mirroring Go's string conversion.
fn decode_utf16(data: &[u8], little: bool) -> Vec<u8> {
    let (chunks, _remainder) = data.as_chunks::<2>();
    let units: Vec<u16> = chunks
        .iter()
        .map(|pair| {
            if little {
                u16::from(pair[0]) | (u16::from(pair[1]) << 8)
            } else {
                (u16::from(pair[0]) << 8) | u16::from(pair[1])
            }
        })
        .collect();
    utf16_decode(&units).into_bytes()
}

fn utf16_decode(units: &[u16]) -> String {
    let mut text = String::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        let c = units[i];
        if (0xD800..=0xDBFF).contains(&c)
            && i + 1 < units.len()
            && (0xDC00..=0xDFFF).contains(&units[i + 1])
        {
            let combined =
                0x10000 + ((u32::from(c) - 0xD800) << 10) + (u32::from(units[i + 1]) - 0xDC00);
            text.push(char::from_u32(combined).expect("valid surrogate pair"));
            i += 2;
        } else {
            match char::from_u32(u32::from(c)) {
                Some(ch) => text.push(ch),
                None => text.push('\u{FFFD}'),
            }
            i += 1;
        }
    }
    text
}

/// Decodes data with a charset, returning the text and how many bytes it
/// consumed. With final false the decoder holds back a trailing partial
/// sequence, so the consumed count marks the whole-character prefix.
fn decode_all(enc: &'static Encoding, data: &[u8], final_: bool) -> String {
    let mut capacity = enc
        .new_decoder()
        .max_utf8_buffer_length(data.len())
        .unwrap_or(0);
    loop {
        let mut text = String::with_capacity(capacity);
        let mut decoder = enc.new_decoder();
        let (result, _read, _written) = decoder.decode_to_string(data, &mut text, final_);
        if result == CoderResult::OutputFull {
            capacity = capacity.saturating_mul(2) + 16;
            continue;
        }
        return text;
    }
}

/// A charset matches when decoding and re-encoding restores the bytes.
/// GB18030 is tried before GBK; the CP936 euro byte only round-trips in
/// GBK, which is how the two stay distinguishable.
fn round_trip(enc: &'static Encoding, data: &[u8], final_: bool) -> Option<(usize, Vec<u8>)> {
    let text = decode_all(enc, data, final_);
    let (back, _encoding, _errors) = enc.encode(&text);
    if final_ {
        if back.as_ref() == data {
            Some((data.len(), text.into_bytes()))
        } else {
            None
        }
    } else {
        // With final false the decoder swallows a trailing partial sequence
        // into its internal state, so its consumed count is not the Go nSrc.
        // The decoded text holds only the complete characters, and a
        // canonical charset re-encodes them to exactly the bytes of the
        // whole-character prefix, so the prefix length is the re-encoded
        // length.
        let n = back.len();
        if data.starts_with(back.as_ref()) {
            Some((n, text.into_bytes()))
        } else {
            None
        }
    }
}

fn sniff(data: &[u8], final_: bool) -> (Kind, usize, Option<Vec<u8>>) {
    if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (Kind::Utf8Bom, data.len(), None);
    }
    if data.starts_with(&[0xFF, 0xFE]) {
        return (Kind::Utf16Le, data.len(), None);
    }
    if data.starts_with(&[0xFE, 0xFF]) {
        return (Kind::Utf16Be, data.len(), None);
    }
    if let Some(kind) = detect_utf16_no_bom(data) {
        return (kind, data.len(), None);
    }
    let whole = if final_ {
        data
    } else {
        trim_partial_rune(data)
    };
    if std::str::from_utf8(whole).is_ok() {
        return (Kind::Utf8, whole.len(), None);
    }
    for (kind, enc) in charsets() {
        if let Some((n, text)) = round_trip(enc, data, final_) {
            return (kind, n, Some(text));
        }
    }
    (Kind::LossyUtf8, data.len(), None)
}

/// Detects the encoding from a BOM prefix only: a cheap first guess for
/// callers that have not buffered enough bytes for a full sniff.
pub fn detect_quick(peek: &[u8]) -> Kind {
    if peek.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Kind::Utf8Bom;
    }
    if peek.starts_with(&[0xFF, 0xFE]) {
        return Kind::Utf16Le;
    }
    if peek.starts_with(&[0xFE, 0xFF]) {
        return Kind::Utf16Be;
    }
    Kind::Utf8
}

/// Converts bytes to UTF-8 under a known encoding. The data must match
/// the encoding (a BOM or UTF-16 payload requires the full prefix);
/// callers holding raw bytes of unknown shape should use
/// detect_and_decode instead.
pub fn decode(data: &[u8], enc: Kind) -> Cow<'_, [u8]> {
    match enc {
        Kind::Utf8 | Kind::LossyUtf8 => Cow::Borrowed(data),
        Kind::Utf8Bom => Cow::Borrowed(&data[3..]),
        Kind::Utf16Le => Cow::Owned(decode_utf16(&data[2..], true)),
        Kind::Utf16Be => Cow::Owned(decode_utf16(&data[2..], false)),
        Kind::Utf16LeNoBom => Cow::Owned(decode_utf16(data, true)),
        Kind::Utf16BeNoBom => Cow::Owned(decode_utf16(data, false)),
        Kind::Gb18030 => Cow::Owned(decode_all(encoding_rs::GB18030, data, true).into_bytes()),
        Kind::Gbk => Cow::Owned(decode_all(encoding_rs::GBK, data, true).into_bytes()),
    }
}

/// Sniffs the encoding of complete file contents and converts them to
/// UTF-8 in one step.
pub fn detect_and_decode(data: &[u8]) -> (Kind, Cow<'_, [u8]>) {
    let (kind, _n, text) = sniff(data, true);
    if let Some(text) = text {
        return (kind, Cow::Owned(text));
    }
    (kind, decode(data, kind))
}

/// Sniffs a prefix of a file and returns the detected kind plus the raw
/// bytes that form whole characters. The caller may re-feed the tail
/// once more input arrives.
pub fn detect_fragment(data: &[u8]) -> (Kind, &[u8]) {
    let (kind, n, _text) = sniff(data, false);
    (kind, &data[..n])
}

/// Detects and converts complete file contents to UTF-8.
pub fn decode_to_utf8(data: &[u8]) -> Cow<'_, [u8]> {
    detect_and_decode(data).1
}

/// Reads a file and converts it to UTF-8 in one step.
pub fn read_file_utf8(path: &Path) -> io::Result<Vec<u8>> {
    let data = fs::read(path)?;
    Ok(decode_to_utf8(&data).into_owned())
}

/// The identity of a write holding a character the file's encoding cannot represent.
pub const ERR_UNENCODABLE: &str = "character not representable in the file's encoding";

/// The first character a legacy charset cannot represent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnencodableError {
    pub charset: &'static str,
    pub rune: char,
    /// Byte offset of `rune` in the text being written.
    pub offset: usize,
    /// Go `%q` of the rune (strconv.Quote on a one-rune string).
    pub quote: String,
}

impl UnencodableError {
    /// Go's `Unwrap` identity, exposed for typed matching.
    pub fn is_unencodable(&self) -> bool {
        true
    }
}

impl std::fmt::Display for UnencodableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (U+{:04X}) at byte {} cannot be written in {}, the file's encoding",
            self.quote, self.rune as u32, self.offset, self.charset
        )
    }
}

impl std::error::Error for UnencodableError {}

/// Encodes UTF-8 text back into the detected file encoding.
pub fn encode(text: &str, kind: Kind) -> Result<Vec<u8>, UnencodableError> {
    encode_bytes(text.as_bytes(), kind)
}

/// Encodes bytes, preserving invalid bytes for the lossy pass-through kind.
pub fn encode_bytes(text: &[u8], kind: Kind) -> Result<Vec<u8>, UnencodableError> {
    match kind {
        Kind::Utf8Bom => {
            let mut out = Vec::with_capacity(text.len() + 3);
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            out.extend_from_slice(text);
            Ok(out)
        }
        Kind::Utf16Le => encode_utf16(text, true, true),
        Kind::Utf16Be => encode_utf16(text, false, true),
        Kind::Utf16LeNoBom => encode_utf16(text, true, false),
        Kind::Utf16BeNoBom => encode_utf16(text, false, false),
        Kind::Gb18030 => encode_charset(text, encoding_rs::GB18030, "GB18030"),
        Kind::Gbk => encode_charset(text, encoding_rs::GBK, "GBK"),
        Kind::Utf8 | Kind::LossyUtf8 => Ok(text.to_vec()),
    }
}

fn encode_utf16(text: &[u8], little: bool, with_bom: bool) -> Result<Vec<u8>, UnencodableError> {
    let body = std::str::from_utf8(text).map_err(|_| unreachable_lossy(text))?;
    let mut out = Vec::with_capacity(body.len() * 2 + 2);
    if with_bom {
        out.extend_from_slice(if little { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
    }
    for unit in body.encode_utf16() {
        if little {
            out.extend_from_slice(&unit.to_le_bytes());
        } else {
            out.extend_from_slice(&unit.to_be_bytes());
        }
    }
    Ok(out)
}

fn unreachable_lossy(text: &[u8]) -> UnencodableError {
    let lossy = String::from_utf8_lossy(text).into_owned();
    let offset = lossy
        .chars()
        .take_while(|c| *c != char::REPLACEMENT_CHARACTER)
        .map(char::len_utf8)
        .sum();
    UnencodableError {
        charset: "UTF-16",
        rune: char::REPLACEMENT_CHARACTER,
        offset,
        quote: go_quote_char(char::REPLACEMENT_CHARACTER),
    }
}

fn encode_charset(
    text: &[u8],
    enc: &'static encoding_rs::Encoding,
    name: &'static str,
) -> Result<Vec<u8>, UnencodableError> {
    let body = std::str::from_utf8(text).map_err(|_| unreachable_lossy(text))?;
    let (cow, _had_bom, had_errors) = enc.encode(body);
    if !had_errors {
        return Ok(cow.into_owned());
    }
    let mut offset = 0usize;
    let mut out = Vec::with_capacity(body.len());
    for c in body.chars() {
        let mut buf = String::with_capacity(4);
        buf.push(c);
        let (part, _enc, errs) = enc.encode(&buf);
        if errs {
            return Err(UnencodableError {
                charset: name,
                rune: c,
                offset,
                quote: go_quote_char(c),
            });
        }
        out.extend_from_slice(&part);
        offset += c.len_utf8();
    }
    Ok(out)
}

fn go_quote_char(c: char) -> String {
    let mut s = String::with_capacity(8);
    s.push('"');
    go_quote_into(c, &mut s);
    s.push('"');
    s
}

fn go_quote_into(c: char, out: &mut String) {
    match c {
        '\\' => out.push_str("\\\\"),
        '"' => out.push_str("\\\""),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        '\x07' => out.push_str("\\a"),
        '\x08' => out.push_str("\\b"),
        '\x0b' => out.push_str("\\v"),
        '\x0c' => out.push_str("\\f"),
        c if is_go_printable(c) => out.push(c),
        c if (c as u32) < 0x100 => out.push_str(&format!("\\x{:02x}", c as u32)),
        c if (c as u32) < 0x10000 => out.push_str(&format!("\\u{:04x}", c as u32)),
        c => out.push_str(&format!("\\U{:08x}", c as u32)),
    }
}

fn is_go_printable(c: char) -> bool {
    if c == ' ' {
        return true;
    }
    !c.is_control() && !c.is_whitespace() && !is_unprintable_format(c)
}

fn is_unprintable_format(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_quick_reports_only_bom_kinds() {
        assert_eq!(detect_quick(&[0xEF, 0xBB, 0xBF, b'a']), Kind::Utf8Bom);
        assert_eq!(detect_quick(&[0xFF, 0xFE, 0x00, 0x68]), Kind::Utf16Le);
        assert_eq!(detect_quick(&[0xFE, 0xFF, 0x00, 0x68]), Kind::Utf16Be);
        assert_eq!(detect_quick(b"plain ascii"), Kind::Utf8);
        assert_eq!(detect_quick(&[0xC4, 0xE3]), Kind::Utf8);
    }

    #[test]
    fn detect_utf16_no_bom_requires_nul_skew() {
        let mut le = Vec::new();
        for b in b"hello world abcdefgh" {
            le.push(*b);
            le.push(0);
        }
        assert_eq!(detect_utf16_no_bom(&le), Some(Kind::Utf16LeNoBom));
        // CJK UTF-16 carries no NUL bytes at all, so nothing is detected.
        assert_eq!(detect_utf16_no_bom(&[0x60, 0x4F]), None);
        // Too short to judge.
        assert_eq!(detect_utf16_no_bom(b"hi\0"), None);
    }

    #[test]
    fn trim_partial_rune_cuts_only_truncated_tails() {
        assert_eq!(trim_partial_rune(b"abc"), b"abc");
        assert_eq!(trim_partial_rune(&[0xE4, 0xBD]), b"");
        assert_eq!(
            trim_partial_rune(&[0xE4, 0xBD, 0xA0, 0xE5]),
            &[0xE4, 0xBD, 0xA0]
        );
        // A complete rune at the tail stays even when earlier bytes are
        // invalid: only a rune straddling the boundary is truncation.
        assert_eq!(trim_partial_rune(&[0xE4, 0xBD, 0x00]), &[0xE4, 0xBD, 0x00]);
        assert_eq!(
            trim_partial_rune(&[0xF0, 0x9F, 0x98, 0x80]),
            &[0xF0, 0x9F, 0x98, 0x80]
        );
        // A four-byte rune cut short is entirely trimmed.
        assert_eq!(trim_partial_rune(&[0xF0, 0x9F, 0x98]), b"");
    }

    #[test]
    fn decode_utf16_handles_surrogates() {
        let data = [
            0xFF, 0xFE, 0x3D, 0xD8, 0x00, 0xDE, 0x20, 0x00, 0x6F, 0x00, 0x6B, 0x00,
        ];
        assert_eq!(decode(&data, Kind::Utf16Le), "😀 ok".as_bytes());
        let lone = [0xFF, 0xFE, 0x00, 0xD8, 0x41, 0x00];
        assert_eq!(decode(&lone, Kind::Utf16Le), "\u{FFFD}A".as_bytes());
    }

    #[test]
    fn gbk_round_trips_the_euro_byte() {
        let (kind, text) = detect_and_decode(&[0x41, 0x80, 0x42]);
        assert_eq!(kind, Kind::Gbk);
        assert_eq!(text.as_ref(), "A\u{20AC}B".as_bytes());
    }

    #[test]
    fn gb18030_wins_four_byte_sequences() {
        let (kind, text) = detect_and_decode(&[0x81, 0x30, 0x81, 0x30]);
        assert_eq!(kind, Kind::Gb18030);
        assert_eq!(text.as_ref(), "\u{80}".as_bytes());
    }

    #[test]
    fn lossy_input_passes_through_untouched() {
        let (kind, text) = detect_and_decode(&[b'a', 0xFF, b'b']);
        assert_eq!(kind, Kind::LossyUtf8);
        assert_eq!(text.as_ref(), &[b'a', 0xFF, b'b']);
    }

    #[test]
    fn read_file_utf8_round_trips() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fileencoding-read-test.txt");
        fs::write(&path, "const x = 1\n").unwrap();
        assert_eq!(read_file_utf8(&path).unwrap(), b"const x = 1\n");
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn core_encode_round_trips_utf8_utf16_and_bom() {
        let cases = [
            (Kind::Utf8, "plain €".as_bytes().to_vec()),
            (Kind::Utf8Bom, b"\xEF\xBB\xBFtitle".to_vec()),
            (Kind::Utf16Le, b"\xFF\xFEh\x00i\x00".to_vec()),
            (Kind::Utf16Be, b"\xFE\xFF\x00h\x00i".to_vec()),
        ];
        for (kind, expected) in cases {
            let text = decode(&expected, kind);
            assert_eq!(encode_bytes(&text, kind).unwrap(), expected);
        }
    }

    #[test]
    fn core_encode_gbk_restores_cp936_euro() {
        let data = [0xC4, 0xE3, 0x80, 0x80, 0x80];
        let text = decode(&data, Kind::Gbk);
        assert_eq!(encode_bytes(&text, Kind::Gbk).unwrap(), data);
    }

    #[test]
    fn core_unencodable_error_preserves_identity_and_first_rune() {
        let error = encode("a😀b", Kind::Gbk).unwrap_err();
        assert!(error.is_unencodable());
        assert_eq!(error.charset, "GBK");
        assert_eq!(error.rune, '😀');
        assert_eq!(error.offset, 1);
        assert_eq!(error.quote, "\"😀\"");
        assert_eq!(
            ERR_UNENCODABLE,
            "character not representable in the file's encoding"
        );
    }

    #[test]
    fn core_encode_bytes_preserves_lossy_input() {
        let data = [b'a', 0xFF, b'\n'];
        assert_eq!(encode_bytes(&data, Kind::LossyUtf8).unwrap(), data);
    }

    #[test]
    fn core_go_quote_char_matches_go_escapes() {
        assert_eq!(go_quote_char('\n'), "\"\\n\"");
        assert_eq!(go_quote_char('\u{1F600}'), "\"😀\"");
        assert_eq!(go_quote_char('\u{200B}'), "\"\\u200b\"");
    }
}
