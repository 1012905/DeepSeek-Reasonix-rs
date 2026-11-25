//! Discovery, caching, and projection of unimported 1.x v4 conversations.

mod cache;
mod discovery;
mod resolver;
mod service;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use reasonix_core::filelock::FileLockError;
use thiserror::Error;

use crate::sessionstore::sessionv4::{Listing, Session as V4Session, Transcript, V4Error};

pub use super::v4_import::{is_v4_session_removed, preserve_v4_removal};
pub use cache::{CACHE_FORMAT, CACHE_NAME, ListingCache};
pub use discovery::{
    LegacyDiscovery, discover_legacy_sessions, legacy_session_info, sort_legacy_rows,
    with_v4_sessions,
};
pub use resolver::{LegacyResolver, resolver_for};
pub use service::{
    AcquireLegacyListingsGuard, LegacyListings, ListingState, ListingStatus, RowState,
    acquire_legacy_listings,
};

/// Why a legacy listing could not be resolved or persisted.
#[derive(Clone, Debug, Error)]
pub enum LegacyListingError {
    #[error("legacy source changed during replay")]
    SourceChanged,
    #[error(transparent)]
    V4(Arc<V4Error>),
    #[error("read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: Arc<io::Error>,
    },
    #[error("decode legacy listing cache: {0}")]
    Json(#[source] Arc<serde_json::Error>),
    #[error("legacy listing cache lock is held")]
    CacheLockHeld,
    #[error("legacy listing cache flush is backing off")]
    Backoff,
    #[error(transparent)]
    Lock(Arc<FileLockError>),
    #[error("legacy listing replay was cancelled")]
    Cancelled,
}

impl From<V4Error> for LegacyListingError {
    fn from(value: V4Error) -> Self {
        Self::V4(Arc::new(value))
    }
}

impl From<io::Error> for LegacyListingError {
    fn from(value: io::Error) -> Self {
        Self::Io {
            path: PathBuf::new(),
            source: Arc::new(value),
        }
    }
}

impl From<serde_json::Error> for LegacyListingError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(Arc::new(value))
    }
}

impl From<FileLockError> for LegacyListingError {
    fn from(value: FileLockError) -> Self {
        match value {
            FileLockError::Held => Self::CacheLockHeld,
            other => Self::Lock(Arc::new(other)),
        }
    }
}

/// Replay override used by the background service and resolver tests.
pub type LegacyReplay = Arc<
    dyn Fn(&V4Session, &CancellationToken) -> Result<Transcript, LegacyListingError>
        + Send
        + Sync
        + 'static,
>;

/// Cooperative cancellation for replay adapters.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    state: Arc<CancelState>,
}

#[derive(Debug, Default)]
struct CancelState {
    cancelled: Mutex<bool>,
    wake: Condvar,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_cancelled(&self) -> bool {
        *self
            .state
            .cancelled
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    pub fn cancel(&self) {
        let mut cancelled = self
            .state
            .cancelled
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        *cancelled = true;
        self.state.wake.notify_all();
    }

    pub fn wait(&self) {
        let mut cancelled = self
            .state
            .cancelled
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        while !*cancelled {
            cancelled = self
                .state
                .wake
                .wait(cancelled)
                .unwrap_or_else(|err| err.into_inner());
        }
    }
}

/// Reads a source transcript, checking cancellation at the synchronous boundary.
pub fn read_legacy_transcript(
    session: &V4Session,
    token: &CancellationToken,
) -> Result<Transcript, LegacyListingError> {
    if token.is_cancelled() {
        return Err(LegacyListingError::Cancelled);
    }
    let transcript = session.transcript()?;
    if token.is_cancelled() {
        return Err(LegacyListingError::Cancelled);
    }
    Ok(transcript)
}

/// The durable tombstone path, or an empty path for a non-import transcript.
pub fn v4_removal_path(session_path: &Path) -> PathBuf {
    crate::store::naming::session_v4_removed_marker(session_path)
}

#[cfg(test)]
#[path = "legacy_listings/tests.rs"]
mod tests;
