//! Frozen wire limits from `docs/contracts/ext.md` §12.1.
//!
//! These constants are part of the protocol: changing one changes the
//! generated schema hash. Both transport directions enforce `FRAME_BYTES`.

/// Maximum bytes in one NDJSON frame, trailing newline included.
pub const FRAME_BYTES: usize = 8 << 20;
/// Inline payloads larger than this must move into a content ref.
pub const EXTERNALIZE_FIELD_BYTES: usize = 64 << 10;
/// Maximum decoded bytes returned by one `host/content/read` page.
pub const CONTENT_REF_CHUNK_BYTES: usize = 256 << 10;
/// Maximum bytes in one externalized content object.
pub const CONTENT_REF_OBJECT_BYTES: usize = 8 << 20;

/// JSON-serializable form of the frozen constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FrozenLimits {
    pub frame_bytes: usize,
    pub externalize_field_bytes: usize,
    pub content_ref_chunk_bytes: usize,
    pub content_ref_object_bytes: usize,
}

/// Returns the immutable limit set for this protocol build.
pub fn frozen_limits() -> FrozenLimits {
    FrozenLimits {
        frame_bytes: FRAME_BYTES,
        externalize_field_bytes: EXTERNALIZE_FIELD_BYTES,
        content_ref_chunk_bytes: CONTENT_REF_CHUNK_BYTES,
        content_ref_object_bytes: CONTENT_REF_OBJECT_BYTES,
    }
}

/// Reports whether a payload must be externalized instead of traveling inline.
pub fn requires_externalization(payload_bytes: usize) -> bool {
    payload_bytes > EXTERNALIZE_FIELD_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_constants_match_contract() {
        assert_eq!(FRAME_BYTES, 8388608);
        assert_eq!(EXTERNALIZE_FIELD_BYTES, 65536);
        assert_eq!(CONTENT_REF_CHUNK_BYTES, 262144);
        assert_eq!(CONTENT_REF_OBJECT_BYTES, 8388608);
        const {
            assert!(EXTERNALIZE_FIELD_BYTES < CONTENT_REF_CHUNK_BYTES);
            assert!(CONTENT_REF_CHUNK_BYTES < CONTENT_REF_OBJECT_BYTES);
            assert!(CONTENT_REF_OBJECT_BYTES <= FRAME_BYTES);
        }
    }

    #[test]
    fn externalization_threshold_is_strict() {
        assert!(!requires_externalization(EXTERNALIZE_FIELD_BYTES));
        assert!(requires_externalization(EXTERNALIZE_FIELD_BYTES + 1));
    }
}
