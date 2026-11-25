//! Legacy v0.x session import.
//!
//! The one-time passes preserve their sources, publish native sessions without
//! replacing existing transcripts, and stamp completion markers only after every
//! session and required sidecar has landed.

mod native;
mod passes;
mod types;
mod util;

#[cfg(test)]
mod tests;

use std::path::Path;

use passes::run_import;
pub use types::{
    LegacyImport, LegacyMigrationError, LegacyReport, LegacySourceError, ProjectDirectory,
    SkipReason, SkippedSession,
};
use util::{ROUTED_CONFIG, ROUTED_HOME, explicit_marker};

/// Imports legacy home-directory sessions and re-homes stranded project sessions.
pub fn migrate_legacy_sessions(
    src_dir: impl AsRef<Path>,
    global_dest: impl AsRef<Path>,
    project_dir: Option<&ProjectDirectory<'_>>,
) -> Result<usize, LegacyMigrationError> {
    let mut report = LegacyReport::default();
    run_import(
        src_dir.as_ref(),
        global_dest.as_ref(),
        ROUTED_HOME,
        util::JSONL_MARKER,
        false,
        project_dir,
        &mut report,
    )
}

/// Imports legacy sessions from an independent config-directory session root.
pub fn migrate_legacy_sessions_from_config_dir(
    src_dir: impl AsRef<Path>,
    global_dest: impl AsRef<Path>,
    project_dir: Option<&ProjectDirectory<'_>>,
) -> Result<usize, LegacyMigrationError> {
    let mut report = LegacyReport::default();
    run_import(
        src_dir.as_ref(),
        global_dest.as_ref(),
        ROUTED_CONFIG,
        util::JSONL_MARKER,
        false,
        project_dir,
        &mut report,
    )
}

/// Imports a user-selected legacy directory, ignoring earlier completion markers.
pub fn import_legacy_sessions_from_explicit_dir(
    src_dir: impl AsRef<Path>,
    global_dest: impl AsRef<Path>,
    project_dir: Option<&ProjectDirectory<'_>>,
) -> Result<LegacyImport, LegacyMigrationError> {
    let src_dir = src_dir.as_ref();
    let marker = explicit_marker(src_dir);
    let mut report = LegacyReport::default();
    let imported = run_import(
        src_dir,
        global_dest.as_ref(),
        &marker,
        &format!("{marker}.jsonl"),
        true,
        project_dir,
        &mut report,
    )?;
    Ok(LegacyImport { imported, report })
}
