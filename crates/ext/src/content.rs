//! Content references: externalization descriptors and paged reads (§3, §12.7).
//!
//! Fields marked externalizable that exceed 64 KiB travel as a null
//! placeholder plus an `ExternalizedField` descriptor; peers page the bytes
//! back with `host/content/read` in 256 KiB chunks, verifying count and hash.

use crate::enums::ContentEncoding;
use crate::limits::{CONTENT_REF_CHUNK_BYTES, CONTENT_REF_OBJECT_BYTES, EXTERNALIZE_FIELD_BYTES};
use serde::{Deserialize, Serialize};

/// Descriptor for one externalizable field that exceeded the inline threshold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalizedField {
    #[serde(rename = "jsonPointer")]
    pub json_pointer: String,
    #[serde(rename = "contentRef")]
    pub content_ref: String,
    #[serde(rename = "totalBytes")]
    pub total_bytes: i64,
    pub sha256: String,
}

impl ExternalizedField {
    /// Validates the descriptor: non-empty identity, byte count, lowercase hash.
    pub fn validate(&self) -> Result<(), ContentValidationError> {
        if self.json_pointer.trim().is_empty() || self.content_ref.trim().is_empty() {
            return Err(ContentValidationError::MissingIdentity);
        }
        if self.total_bytes < 0 {
            return Err(ContentValidationError::NegativeBytes);
        }
        if self.total_bytes as usize > CONTENT_REF_OBJECT_BYTES {
            return Err(ContentValidationError::ObjectTooLarge);
        }
        if !is_lowercase_sha256(&self.sha256) {
            return Err(ContentValidationError::BadHash);
        }
        Ok(())
    }
}

/// Request one page of a content ref.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentReadParams {
    #[serde(rename = "contentRef")]
    pub content_ref: String,
    pub offset: i64,
}

/// One page of content ref data, with whole-object verification fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentReadResult {
    #[serde(rename = "contentRef")]
    pub content_ref: String,
    pub offset: i64,
    #[serde(rename = "dataBase64")]
    pub data_base64: String,
    #[serde(
        rename = "nextOffset",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub next_offset: Option<i64>,
    #[serde(rename = "totalBytes")]
    pub total_bytes: i64,
    pub sha256: String,
    pub encoding: ContentEncoding,
}

impl ContentReadResult {
    /// Validates the page: non-negative offsets, bounded chunk, valid hash.
    pub fn validate(&self) -> Result<(), ContentValidationError> {
        if self.offset < 0 || self.next_offset.is_some_and(|n| n < 0) {
            return Err(ContentValidationError::NegativeBytes);
        }
        if decoded_len(&self.data_base64) > CONTENT_REF_CHUNK_BYTES {
            return Err(ContentValidationError::ChunkTooLarge);
        }
        if self.total_bytes < 0 || self.total_bytes as usize > CONTENT_REF_OBJECT_BYTES {
            return Err(ContentValidationError::ObjectTooLarge);
        }
        if !is_lowercase_sha256(&self.sha256) {
            return Err(ContentValidationError::BadHash);
        }
        Ok(())
    }
}

/// Why a content DTO failed validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContentValidationError {
    #[error("content descriptor is missing identity")]
    MissingIdentity,
    #[error("content byte count must be non-negative")]
    NegativeBytes,
    #[error("content page exceeds the chunk limit")]
    ChunkTooLarge,
    #[error("content object exceeds 8 MiB")]
    ObjectTooLarge,
    #[error("sha256 must be lowercase hex of 64 characters")]
    BadHash,
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn decoded_len(base64: &str) -> usize {
    base64.len() / 4 * 3
}

/// Reports whether a payload needs externalization under the frozen threshold.
pub fn must_externalize(payload_bytes: usize) -> bool {
    payload_bytes > EXTERNALIZE_FIELD_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field() -> ExternalizedField {
        ExternalizedField {
            json_pointer: "/payload".into(),
            content_ref: "cref-1".into(),
            total_bytes: 2,
            sha256: "a".repeat(64),
        }
    }

    #[test]
    fn externalized_field_validates() {
        assert!(field().validate().is_ok());
        let mut bad = field();
        bad.sha256 = "A".repeat(64);
        assert_eq!(bad.validate(), Err(ContentValidationError::BadHash));
        let mut bad = field();
        bad.total_bytes = -1;
        assert_eq!(bad.validate(), Err(ContentValidationError::NegativeBytes));
        let mut bad = field();
        bad.content_ref.clear();
        assert_eq!(bad.validate(), Err(ContentValidationError::MissingIdentity));
    }

    #[test]
    fn content_read_round_trip_and_limits() {
        let page = ContentReadResult {
            content_ref: "cref-1".into(),
            offset: 0,
            data_base64: "aGk=".into(),
            next_offset: None,
            total_bytes: 2,
            sha256: "b".repeat(64),
            encoding: ContentEncoding::Utf8,
        };
        assert!(page.validate().is_ok());
        let json = serde_json::to_string(&page).unwrap();
        assert!(json.contains("\"encoding\":\"utf8\""));
        assert!(json.contains("\"dataBase64\":\"aGk=\""));
        let decoded: ContentReadResult = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, page);
        let mut bad = page;
        bad.next_offset = Some(-1);
        assert_eq!(bad.validate(), Err(ContentValidationError::NegativeBytes));
    }

    #[test]
    fn threshold_and_strict_dto_decode() {
        assert!(!must_externalize(EXTERNALIZE_FIELD_BYTES));
        assert!(must_externalize(EXTERNALIZE_FIELD_BYTES + 1));
        let params: ContentReadParams =
            serde_json::from_str("{\"contentRef\":\"c\",\"offset\":0}").expect("valid params");
        assert_eq!(params.content_ref, "c");
        assert!(
            serde_json::from_str::<ContentReadParams>(
                "{\"contentRef\":\"c\",\"offset\":0,\"extra\":2}"
            )
            .is_err(),
            "strict DTO decode rejects unknown members"
        );
    }
}
