//! ACP-042: session id validation (missing / invalid / charset / dot segments /
//! surrounding whitespace) and the injected-RNG RFC 4122 v4 layout.

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::session_id::{
    SessionIdSource, is_safe_session_id, is_uuid_v4, new_session_id, uuid_v4_from_bytes,
    validate_session_id,
};

#[test]
fn valid_ids_pass_and_dot_segments_fail() {
    assert!(validate_session_id("session/new", "abc-DEF_123.x").is_ok());
    assert!(validate_session_id("session/new", ".").is_err());
    assert!(validate_session_id("session/new", "..").is_err());
    assert!(is_safe_session_id("a-zA-Z0-9._-"));
}

#[test]
fn missing_blank_id_uses_the_missing_session_id_message() {
    let err = validate_session_id("session/list", "  ").unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidParams);
    assert_eq!(err.message, "session/list: missing sessionId");
}

#[test]
fn surrounding_whitespace_and_illegal_chars_use_the_invalid_message() {
    let leading = validate_session_id("session/new", " s1").unwrap_err();
    assert_eq!(leading.message, "session/new: invalid sessionId");
    let trailing = validate_session_id("session/new", "s1 ").unwrap_err();
    assert_eq!(trailing.message, "session/new: invalid sessionId");
    let illegal = validate_session_id("session/list", "s1/../x").unwrap_err();
    assert_eq!(illegal.message, "session/list: invalid sessionId");
    assert!(!is_safe_session_id("s1/../x"));
}

#[test]
fn v4_format_sets_version_and_variant_bits_and_lowercase_hex() {
    let id = uuid_v4_from_bytes(&[0u8; 16]);
    assert_eq!(id, "00000000-0000-4000-8000-000000000000");
    assert!(is_uuid_v4(&id));

    let all_ff = uuid_v4_from_bytes(&[0xff; 16]);
    assert_eq!(all_ff, "ffffffff-ffff-4fff-bfff-ffffffffffff");
    assert!(is_uuid_v4(&all_ff));
}

struct FixedSource([u8; 16]);
impl SessionIdSource for FixedSource {
    fn next_bytes(&mut self) -> [u8; 16] {
        self.0
    }
}

#[test]
fn injected_source_allocates_a_v4_session_id() {
    let mut source = FixedSource([0x12; 16]);
    let id = new_session_id(&mut source);
    assert!(is_uuid_v4(&id));
    assert_eq!(id, "12121212-1212-4212-9212-121212121212");
}

#[test]
fn uuid_validator_rejects_wrong_shape_version_variant_and_uppercase() {
    assert!(!is_uuid_v4("00000000-0000-4000-8000-00000000000"));
    assert!(!is_uuid_v4("00000000-0000-5000-8000-000000000000"));
    assert!(!is_uuid_v4("00000000-0000-4000-c000-000000000000"));
    assert!(!is_uuid_v4("00000000-0000-4000-8000-00000000000G"));
    assert!(!is_uuid_v4("00000000-0000-4000-8000-00000000000"));
    assert!(is_uuid_v4("00000000-0000-4000-a000-000000000000"));
}
