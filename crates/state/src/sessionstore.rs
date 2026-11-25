//! Session-store level paths and naming.
//!
//! Parity target: `reference/.../internal/state/sessionstore`
//! (docs/contracts/session-format.md 2).

pub mod branch;
pub mod cleanup_pending;
pub mod compaction_state;
pub mod composition_archive;
pub mod dag;
pub mod display_index;
pub mod display_read_model;
pub mod eventlog;
pub mod legacy_listings;
pub mod legacy_migration;
pub mod listing;
pub mod path;
pub mod preview;
pub mod recovery_archive;
pub mod recovery_gc;
pub mod recovery_lineage;
pub mod recovery_trash;
pub mod save_conflict;
pub mod session;
pub mod session_lease;
pub mod session_lock;
pub mod session_order;
pub mod session_range;
pub mod session_removal;
pub mod sessionpreview;
pub mod sessionv4;
pub mod turn;
pub mod v4_import;
pub mod versions;

pub use path::{new_session_path, new_session_path_at};
pub use session::{
    PersistedState, Session, SessionPersistEvent, SessionPersistObserver, canonical_session_path,
    digest_and_size_session_messages, digest_session_messages,
};
