//! Snapshot checkpoint storage.

mod atomic_json;
mod barrier;
mod blob;
mod conversation_fork;
mod observer;
mod ownership;
mod path_capture;
mod rewind;
mod secure_path;
mod store;
mod transaction;
mod transaction_compensate;
mod transaction_prepare;
mod transaction_publish;
mod transaction_recovery;
mod types;
mod undo;
mod writer_admission;

pub use atomic_json::{read_json_file, write_json_atomic};
pub use barrier::{BarrierError, ExclusivePermit, MutationBarrier, WritePermit};
pub use blob::{BlobError, BlobStore, digest, valid_blob_ref};
pub use conversation_fork::ConversationForkError;
pub use observer::{BeginWriteError, MutationObserver, ObserverOptions};
pub use path_capture::{
    CaptureError, Fingerprint, PathError, capture_path, compare_identity, fingerprint_path,
    normalize_rel_path, safe_path,
};
pub use reasonix_core::fileencoding;
pub use rewind::RewindError;
pub use secure_path::{
    SecureError, chmod, open_workspace_file, path_exists, read_file, remove, rename, write_new_file,
};
pub use store::{Store, StoreConfig, StoreError, expired_dir, turn_path};
pub use transaction::{
    ConversationApplier, ConversationError, ConversationPhase, RecoveryNote, RecoveryPhase,
    TransactionError, TransactionInjection, TransactionPhase,
};
pub use types::*;
pub use writer_admission::{RegisteredWriter, WriterError, WriterErrorKind};

#[cfg(test)]
mod barrier_tests;
#[cfg(test)]
mod blob_path_tests;
#[cfg(test)]
mod conversation_fork_tests;
#[cfg(test)]
mod gc_tests;
#[cfg(test)]
mod observer_tests;
#[cfg(test)]
mod ownership_tests;
#[cfg(test)]
mod rewind_tests;
#[cfg(test)]
mod secure_path_tests;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod transaction_conversation_tests;
#[cfg(test)]
mod transaction_recovery_tests;
#[cfg(test)]
mod transaction_test_support;
#[cfg(test)]
mod transaction_tests;
#[cfg(test)]
mod undo_tests;
#[cfg(test)]
mod wire_tests;
#[cfg(test)]
mod writer_admission_tests;
