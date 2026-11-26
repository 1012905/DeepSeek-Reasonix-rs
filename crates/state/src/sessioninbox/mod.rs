//! Durable session-inbox wire types, manifest validation, and disk ownership.
//! Queue and admission operations are intentionally absent.

mod blob;
mod disk;
mod disk_blob;
mod disk_fs;
mod disk_manifest;
mod disk_recovery;
mod ids;
mod manifest;
mod types;

pub use disk::{Store, remove_dir};
pub use ids::{preview_text, process_run_id, sha256_hex};
pub use types::*;

#[cfg(test)]
pub(crate) use blob::encode_envelope;
pub(crate) use blob::idempotency_request_hash;

pub use manifest::{
    IDEMPOTENCY_RECEIPT_TTL, IdempotencyReceipt, MAX_IDEMPOTENCY_KEY_BYTES,
    MAX_IDEMPOTENCY_RECEIPTS, Manifest, blob_name_for, decode_manifest, empty_manifest,
    valid_blob_stem, valid_idempotency_key, valid_sha256, validate_manifest,
};

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod blob_tests;
#[cfg(test)]
mod disk_recovery_tests;
#[cfg(test)]
mod disk_tests;
#[cfg(test)]
mod ids_tests;
#[cfg(test)]
mod manifest_tests;
#[cfg(test)]
mod types_tests;
