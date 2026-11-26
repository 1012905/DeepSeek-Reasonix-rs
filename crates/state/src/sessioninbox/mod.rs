//! Durable session-inbox wire types and manifest foundations.
//! Disk ownership and queue operations are intentionally absent.

mod ids;
mod manifest;
mod types;

pub use ids::{preview_text, process_run_id, sha256_hex};
pub use types::*;

pub use manifest::{
    IDEMPOTENCY_RECEIPT_TTL, IdempotencyReceipt, MAX_IDEMPOTENCY_KEY_BYTES,
    MAX_IDEMPOTENCY_RECEIPTS, Manifest, blob_name_for, decode_manifest, empty_manifest,
    valid_blob_stem, valid_idempotency_key, valid_sha256, validate_manifest,
};

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod ids_tests;
#[cfg(test)]
mod manifest_tests;
#[cfg(test)]
mod types_tests;
