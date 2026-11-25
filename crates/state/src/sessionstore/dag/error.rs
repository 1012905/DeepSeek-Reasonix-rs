use thiserror::Error;

use crate::sessionstore::eventlog::SessionReplayLimitError;

#[derive(Debug, Error)]
pub enum DagError {
    #[error("decode session event log {path}: unsupported schema version {version}")]
    UnsupportedSchema { path: String, version: i64 },
    #[error("decode session event log {path}: unsupported entry type {entry_type:?}")]
    UnsupportedEntry { path: String, entry_type: String },
    #[error(transparent)]
    Limit(#[from] SessionReplayLimitError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
