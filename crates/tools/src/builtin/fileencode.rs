//! The encode side of the file-encoding cascade (encoding.go `Encode`):
//! UTF-8 text back to the encoding a file was detected in, so an edit
//! rewrites the file in its original charset instead of switching it to
//! UTF-8. The decode side lives in reasonix-core::fileencoding, which
//! anchors this half to the file-tools slice.

#![forbid(unsafe_code)]

use reasonix_core::fileencoding::Kind;

/// encoding.go `ErrUnencodable`: the identity of a write holding a
/// character the file's encoding cannot represent.
pub const ERR_UNENCODABLE: &str = "character not representable in the file's encoding";

/// encoding.go `UnencodableError`: names the first character a legacy
/// charset cannot represent.
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

/// encoding.go `Encode` for arbitrary text: UTF-8 and LossyUTF8 pass
/// through; a BOM is re-added where the kind had one; UTF-16 re-encodes
/// code units; a legacy charset answers [`UnencodableError`] rather than
/// switching the file to UTF-8.
pub fn encode(text: &str, kind: Kind) -> Result<Vec<u8>, UnencodableError> {
    encode_bytes(text.as_bytes(), kind)
}

/// The byte form of `Encode`, so the edit engine can round-trip content it
/// holds as bytes (a LossyUTF8 file's bytes are not valid UTF-8 and pass
/// through untouched).
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
        // UTF8 and LossyUTF8 both pass through — LossyUTF8 is already
        // "best effort" and keeps the bytes it did not edit.
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

// A UTF-16/GBK target's content came from a decode that produces valid
// UTF-8, so an invalid sequence means a caller bypassed the source route.
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

/// encoding.go `encodeCharset`: encode, and on failure name the first
/// character the charset cannot represent. GB18030/GBK are table codecs —
/// per-character encoding is context-free — so the failing character is
/// found by encoding a prefix incrementally once the whole-string pass
/// reports an error.
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
    // Slow path only: find the first unmappable character and its byte
    // offset by encoding one character at a time with fresh encoders.
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

/// strconv.Quote for a single rune: printable runes kept, the rest escaped
/// the way Go renders them in the refusal text.
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

/// Go unicode.IsPrint, close enough for the refuse-text quoting: printable
/// runes are graphic plus space. The exact table edge differences only
/// affect how a refusal names an exotic character, never its identity.
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
    use reasonix_core::fileencoding::{decode, detect_and_decode};

    #[test]
    fn encode_round_trips_detected_kinds() {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("utf8", "你好世界\n".as_bytes().to_vec()),
            ("utf8-bom", {
                let mut v = vec![0xEF, 0xBB, 0xBF];
                v.extend_from_slice("标题：旧的说明\r\n".as_bytes());
                v
            }),
            ("utf16le", {
                let mut v = vec![0xFF, 0xFE];
                for u in "第二行".encode_utf16() {
                    v.extend_from_slice(&u.to_le_bytes());
                }
                v
            }),
        ];
        for (label, data) in cases {
            let (kind, text) = detect_and_decode(&data);
            let back = encode_bytes(&text, kind).unwrap();
            assert_eq!(back, data, "{label} round trip");
        }
    }

    #[test]
    fn gbk_round_trip_restores_cp936_euro_byte() {
        // 0x80 is CP936's euro; GB18030 cannot restore it, GBK can — the
        // same distinguishability the detector relies on.
        let data = vec![0xC4, 0xE3, 0x80, 0x80, 0x80];
        let decoded = decode(&data, Kind::Gbk).into_owned();
        let back = encode_bytes(&decoded, Kind::Gbk).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn unencodable_error_names_first_character() {
        let err = encode("a\u{1F600}b", Kind::Gbk).unwrap_err();
        assert_eq!(err.charset, "GBK");
        assert_eq!(err.rune, '\u{1F600}');
        assert_eq!(err.offset, 1);
        let rendered = err.to_string();
        assert!(rendered.contains("cannot be written in GBK"), "{rendered}");
        assert!(rendered.contains(ERR_UNENCODABLE) || rendered.starts_with('"'));
    }

    #[test]
    fn lossy_kind_keeps_invalid_bytes() {
        let data: Vec<u8> = vec![0x61, 0xFF, 0x0A];
        let (kind, text) = detect_and_decode(&data);
        assert_eq!(kind, Kind::LossyUtf8);
        assert_eq!(encode_bytes(&text, kind).unwrap(), data);
    }
}
