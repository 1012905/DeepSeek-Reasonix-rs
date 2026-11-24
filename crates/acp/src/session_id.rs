//! ACP-042: session id validation and RFC 4122 v4 generation
//! (`service.go:1422-1450,1547-1556`).
//!
//! The id charset, the surrounding-whitespace and dot-segment refusals, and the
//! v4 layout/version/variant bits are pure. The random 16-byte source is a
//! runtime seam; tests supply fixed bytes.

use crate::errors::RpcError;

/// Valid session id characters: ASCII letters, digits, `-`, `_`, `.`
/// (`service.go:1433-1450`).
fn is_safe_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'
}

/// Charset check alone; callers combine it with the whitespace and dot-segment
/// rules (`service.go:1433-1450`).
pub fn is_safe_session_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(is_safe_char)
}

/// Validates a session id for a request method. Empty after trimming is
/// `missing sessionId`; surrounding whitespace, the `.` / `..` segments, and
/// any outside the charset are `invalid sessionId` (`service.go:1422-1431`).
pub fn validate_session_id(method: &str, id: &str) -> Result<(), RpcError> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(RpcError::invalid_params(format!(
            "{method}: missing sessionId"
        )));
    }
    if trimmed != id || trimmed == "." || trimmed == ".." || !is_safe_session_id(trimmed) {
        return Err(RpcError::invalid_params(format!(
            "{method}: invalid sessionId"
        )));
    }
    Ok(())
}

/// Applies the RFC 4122 version-4 and variant bits to one octet block
/// (`service.go:1553-1554`).
fn apply_version_variant(mut bytes: [u8; 16]) -> [u8; 16] {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    bytes
}

/// Lowercase hex pairs for one byte.
fn hex_pair(byte: u8) -> String {
    format!("{byte:02x}")
}

/// Hex string for a byte range.
fn hex_range(bytes: &[u8]) -> String {
    bytes.iter().map(|b| hex_pair(*b)).collect()
}

/// Formats 16 random bytes as a lowercase v4 UUID, `8-4-4-4-12`
/// (`service.go:1548-1556`).
pub fn uuid_v4_from_bytes(bytes: &[u8; 16]) -> String {
    let b = apply_version_variant(*bytes);
    format!(
        "{}-{}-{}-{}-{}",
        hex_range(&b[0..4]),
        hex_range(&b[4..6]),
        hex_range(&b[6..8]),
        hex_range(&b[8..10]),
        hex_range(&b[10..16])
    )
}

/// Validates an RFC 4122 v4 UUID: shape `8-4-4-4-12` lowercase hex, version
/// nibble `4`, variant nibble `8`–`b`.
pub fn is_uuid_v4(candidate: &str) -> bool {
    let parts: Vec<&str> = candidate.split('-').collect();
    let lengths = [8, 4, 4, 4, 12];
    if parts.len() != 5 || parts.iter().zip(lengths).any(|(p, n)| p.len() != n) {
        return false;
    }
    if !parts.iter().all(|p| {
        p.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }) {
        return false;
    }
    let version = parts[2].as_bytes()[0];
    let variant = parts[3].as_bytes()[0];
    version == b'4' && matches!(variant, b'8' | b'9' | b'a' | b'b')
}

/// Supplies the 16 random bytes for a new session id. The runtime backs this
/// with the platform CSPRNG; tests inject fixed bytes.
pub trait SessionIdSource {
    fn next_bytes(&mut self) -> [u8; 16];
}

/// Allocates a new v4 session id (`service.go:1547-1556`).
pub fn new_session_id(source: &mut dyn SessionIdSource) -> String {
    uuid_v4_from_bytes(&source.next_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcErrorCode;

    struct FixedSource([u8; 16]);
    impl SessionIdSource for FixedSource {
        fn next_bytes(&mut self) -> [u8; 16] {
            self.0
        }
    }

    #[test]
    fn safe_ids_pass_and_whitespace_dot_segments_fail() {
        assert!(validate_session_id("session/new", "abc-DEF_123.45").is_ok());
        assert!(validate_session_id("session/new", "..").is_err());
        assert!(validate_session_id("session/new", ".").is_err());

        let leading = validate_session_id("session/new", " s1").unwrap_err();
        assert_eq!(leading.code, RpcErrorCode::InvalidParams);
        assert_eq!(leading.message, "session/new: invalid sessionId");

        let trailing = validate_session_id("session/new", "s1 ").unwrap_err();
        assert_eq!(trailing.message, "session/new: invalid sessionId");
    }

    #[test]
    fn missing_and_illegal_chars_use_distinct_messages() {
        let missing = validate_session_id("session/list", "  ").unwrap_err();
        assert_eq!(missing.code, RpcErrorCode::InvalidParams);
        assert_eq!(missing.message, "session/list: missing sessionId");

        let illegal = validate_session_id("session/list", "s1/../x").unwrap_err();
        assert_eq!(illegal.message, "session/list: invalid sessionId");
        assert!(!is_safe_session_id("s1/../x"));
        assert!(is_safe_session_id("a-zA-Z0-9._-"));
    }

    #[test]
    fn uuid_v4_layout_bits_and_lowercase_are_frozen() {
        let bytes = [0u8; 16];
        let id = uuid_v4_from_bytes(&bytes);
        assert_eq!(id, "00000000-0000-4000-8000-000000000000");
        assert!(is_uuid_v4(&id));

        let mut source = FixedSource([0xff; 16]);
        let allocated = new_session_id(&mut source);
        assert_eq!(allocated, "ffffffff-ffff-4fff-bfff-ffffffffffff");
        assert!(is_uuid_v4(&allocated));
    }

    #[test]
    fn uuid_validator_rejects_wrong_shape_version_and_variant() {
        assert!(!is_uuid_v4("00000000-0000-4000-8000-00000000000"));
        assert!(!is_uuid_v4("00000000-0000-5000-8000-000000000000"));
        assert!(!is_uuid_v4("00000000-0000-4000-c000-000000000000"));
        assert!(!is_uuid_v4("00000000-0000-4000-8000-00000000000G"));
        assert!(!is_uuid_v4("00000000-0000-4000-8000-00000000000"));
        assert!(is_uuid_v4("00000000-0000-4000-a000-000000000000"));
    }
}
