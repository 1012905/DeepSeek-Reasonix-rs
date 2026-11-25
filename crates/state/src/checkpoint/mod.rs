//! Snapshot checkpoint storage.

mod blob;
mod path_capture;
mod store;
mod types;

pub use blob::{BlobError, BlobStore, digest, valid_blob_ref};
pub use path_capture::{
    CaptureError, Fingerprint, PathError, capture_path, compare_identity, fingerprint_path,
    normalize_rel_path, safe_path,
};
pub use reasonix_core::fileencoding;
pub use store::{Store, StoreConfig, StoreError, expired_dir, turn_path};
pub use types::*;

#[cfg(test)]
mod blob_path_tests;
#[cfg(test)]
mod gc_tests;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod wire_tests;
