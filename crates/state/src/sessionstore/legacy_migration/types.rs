use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error as ThisError;

use crate::sessionstore::branch::BranchMetaError;
use crate::sessionstore::eventlog::ReplayError;
use crate::sessionstore::session::{LoadError, SaveError};

/// Routes a live workspace to its native per-project session directory.
pub type ProjectDirectory<'a> = dyn Fn(&Path) -> Option<PathBuf> + Send + Sync + 'a;

/// The stable class of one legacy entry that was not imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    TooLarge,
    UnreadableFormat,
    SchemaUnsupported,
    Permission,
    Corrupt,
    CopyFailed,
    NoProject,
}

impl SkipReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TooLarge => "too_large",
            Self::UnreadableFormat => "unreadable_format",
            Self::SchemaUnsupported => "schema_unsupported",
            Self::Permission => "permission",
            Self::Corrupt => "corrupt",
            Self::CopyFailed => "copy_failed",
            Self::NoProject => "no_project",
        }
    }
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One source entry left untouched by an explicit import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedSession {
    pub name: String,
    pub path: PathBuf,
    pub reason: SkipReason,
}

/// An implicit import could not list its source directory.
#[derive(Debug, ThisError)]
#[error("could not read legacy sessions directory {}: {source}", .skipped.path.display())]
pub struct LegacySourceError {
    pub skipped: SkippedSession,
    #[source]
    pub source: io::Error,
}

/// Failures that abort an implicit migration instead of becoming per-entry skips.
#[derive(Debug, ThisError)]
pub enum LegacyMigrationError {
    #[error(transparent)]
    Source(#[from] LegacySourceError),
}

/// Entries an explicit import found but could not bring over.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyReport {
    pub skipped: Vec<SkippedSession>,
}

impl LegacyReport {
    pub(crate) fn skip(&mut self, path: &Path, reason: SkipReason) {
        self.skipped.push(SkippedSession {
            name: file_name(path),
            path: path.to_path_buf(),
            reason,
        });
    }

    pub(crate) fn skip_error(&mut self, path: &Path, error: &OperationError) {
        self.skip(path, error.read_reason());
    }

    pub(crate) fn skip_write(&mut self, path: &Path, error: &OperationError) {
        self.skip(
            path,
            if error.is_permission() {
                SkipReason::Permission
            } else {
                SkipReason::CopyFailed
            },
        );
    }
}

/// Result of an explicit-directory import.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyImport {
    pub imported: usize,
    pub report: LegacyReport,
}

#[derive(Debug, ThisError)]
pub(crate) enum OperationError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Load(#[from] LoadError),
    #[error(transparent)]
    Save(#[from] SaveError),
    #[error(transparent)]
    Branch(#[from] BranchMetaError),
    #[error("session destination already exists")]
    Exists,
    #[error("source is not a message-format transcript")]
    UnreadableFormat,
}

impl OperationError {
    pub(crate) fn is_permission(&self) -> bool {
        error_is_permission(self)
    }

    pub(crate) fn read_reason(&self) -> SkipReason {
        if self.is_permission() {
            return SkipReason::Permission;
        }
        match self {
            Self::Load(LoadError::Replay(ReplayError::Limit(_))) => SkipReason::TooLarge,
            Self::Load(LoadError::Replay(ReplayError::UnsupportedSchemaVersion { .. })) => {
                SkipReason::SchemaUnsupported
            }
            Self::UnreadableFormat => SkipReason::UnreadableFormat,
            _ => SkipReason::Corrupt,
        }
    }
}

fn error_is_permission(error: &(dyn Error + 'static)) -> bool {
    let mut current: Option<&(dyn Error + 'static)> = Some(error);
    while let Some(error) = current {
        if let Some(io) = error.downcast_ref::<io::Error>()
            && io.kind() == io::ErrorKind::PermissionDenied
        {
            return true;
        }
        current = error.source();
    }
    false
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod skip_tests {
    use super::*;
    use crate::sessionstore::eventlog::SessionReplayLimitError;

    #[test]
    fn replay_limits_and_schema_errors_keep_their_identity() {
        let limit = SessionReplayLimitError {
            path: "session".into(),
            resource: "event_records",
            value: 2,
            limit: 1,
        };
        let error = OperationError::Load(LoadError::Replay(ReplayError::Limit(limit)));
        assert_eq!(error.read_reason(), SkipReason::TooLarge);
        let error =
            OperationError::Load(LoadError::Replay(ReplayError::UnsupportedSchemaVersion {
                path: "session".into(),
                version: 99,
            }));
        assert_eq!(error.read_reason(), SkipReason::SchemaUnsupported);
    }
}
