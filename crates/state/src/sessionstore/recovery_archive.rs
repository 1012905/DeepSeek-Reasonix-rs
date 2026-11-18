//! Recovery-lineage archive transactions (Go: recovery_archive.go).
//!
//! Archiving a recovery branch must archive its whole lineage, and a
//! lineage is only as trustworthy as its weakest sidecar: when any
//! listed sibling's metadata cannot be read, the transaction is
//! refused rather than half-applied. Writes are applied under the
//! session-meta locks of every touched path and rolled back on the
//! first failure (Go: `setSessionPathsArchivedWith` +
//! `rollbackArchivedMetaSnapshots`).
//!
//! Contract: docs/contracts/session-format.md SS15.4.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use super::branch::{
    BranchMeta, BranchMetaError, branch_id, ensure_branch_meta_unlocked, load_branch_meta,
    lock_session_meta_path, save_branch_meta_preserve_updated_locked, set_session_archived,
};
use super::recovery_gc::go_dir;
use super::session::canonical_session_path;
use super::session_order::{SessionOrderInfo, list_session_order};
use crate::store::naming;

/// Cap on the recovery-root walk (Go: `maxRecoveryArchiveLineageWalk`).
const MAX_RECOVERY_ARCHIVE_LINEAGE_WALK: usize = 16;

/// Failure of a lineage-archive transaction.
#[derive(Debug)]
pub enum LineageArchiveError {
    /// Listing the sibling metas failed.
    ListLineage {
        /// Basename of the archive target.
        base: String,
        /// Newline-joined unreadable-sidecar errors (Go errors.Join).
        joined: String,
    },
    /// An archive write failed after some siblings were already
    /// written; `rollback` describes any rollback failures.
    Archive {
        /// The write failure that aborted the transaction.
        source: BranchMetaError,
        /// Newline-joined rollback failures, if any.
        rollback: Option<String>,
    },
    /// A branch-meta operation failed.
    Meta(BranchMetaError),
    /// Filesystem access failed.
    Io(io::Error),
}

impl fmt::Display for LineageArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LineageArchiveError::ListLineage { base, joined } => {
                write!(f, "list recovery lineage for {base}: {joined}")
            }
            LineageArchiveError::Archive { source, rollback } => {
                write!(f, "archive session lineage: {source}")?;
                if let Some(rollback) = rollback {
                    writeln!(f)?;
                    write!(f, "{rollback}")?;
                }
                Ok(())
            }
            LineageArchiveError::Meta(err) => write!(f, "{err}"),
            LineageArchiveError::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for LineageArchiveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LineageArchiveError::Archive { source, .. } => Some(source),
            LineageArchiveError::Meta(err) => Some(err),
            LineageArchiveError::Io(err) => Some(err),
            LineageArchiveError::ListLineage { .. } => None,
        }
    }
}

impl From<BranchMetaError> for LineageArchiveError {
    fn from(err: BranchMetaError) -> Self {
        LineageArchiveError::Meta(err)
    }
}

impl From<io::Error> for LineageArchiveError {
    fn from(err: io::Error) -> Self {
        LineageArchiveError::Io(err)
    }
}

/// Meta snapshot taken before an archive write, for rollback (Go:
/// `archivedMetaSnapshot`).
struct ArchivedSnapshot {
    path: String,
    meta: BranchMeta,
    existed: bool,
}

/// Archive the whole recovery lineage of `session_path` (Go:
/// `SetSessionLineageArchived`). A single-path lineage delegates
/// to `set_session_archived`; wider lineages lock every path's
/// meta file first and roll back all writes if any save fails.
pub fn set_session_lineage_archived(
    session_path: &str,
    archived: bool,
    except: &[&str],
) -> Result<(), LineageArchiveError> {
    let paths = recovery_archive_lineage(session_path, except)?;
    if paths.len() == 1 {
        set_session_archived(Path::new(&paths[0]), archived)?;
        return Ok(());
    }
    set_session_paths_archived_with(&paths, archived, &mut default_archive_save)
}

/// Paths of the recovery lineage of `session_path`, excluding
/// `except` (Go: `RecoveryLineagePaths`).
pub fn recovery_lineage_paths(
    session_path: &str,
    except: &[&str],
) -> Result<Vec<String>, LineageArchiveError> {
    recovery_archive_lineage(session_path, except)
}

/// Archive variant with an injected save function for tests (Go:
/// `setSessionLineageArchivedWith`).
pub fn set_session_lineage_archived_with(
    session_path: &str,
    archived: bool,
    except: &[&str],
    save: &mut dyn FnMut(&Path, BranchMeta) -> Result<(), BranchMetaError>,
) -> Result<(), LineageArchiveError> {
    let paths = recovery_archive_lineage(session_path, except)?;
    set_session_paths_archived_with(&paths, archived, save)
}

/// Production save: keep `updated_at` while flipping
/// `archived` (Go: `saveBranchMeta` with
/// `touchUpdated=false`).
fn default_archive_save(path: &Path, meta: BranchMeta) -> Result<(), BranchMetaError> {
    let mut meta = meta;
    save_branch_meta_preserve_updated_locked(path, &mut meta)
}

/// Lock-then-write archive transaction (Go:
/// `setSessionPathsArchivedWith`).
fn set_session_paths_archived_with(
    paths: &[String],
    archived: bool,
    save: &mut dyn FnMut(&Path, BranchMeta) -> Result<(), BranchMetaError>,
) -> Result<(), LineageArchiveError> {
    // Go locks every meta file up front and unlocks backward on
    // return. Each lock file is independent, so guard drop order does
    // not matter; the guards below release on every exit path.
    let mut locks = Vec::with_capacity(paths.len());
    for path in paths {
        let guard = lock_session_meta_path(Path::new(path))?;
        locks.push(guard);
    }
    let mut snapshots: Vec<ArchivedSnapshot> = Vec::with_capacity(paths.len());
    for path in paths {
        let (meta, existed) = match load_branch_meta(Path::new(path))? {
            Some(meta) => (meta, true),
            None => (ensure_branch_meta_unlocked(Path::new(path))?, false),
        };
        snapshots.push(ArchivedSnapshot {
            path: path.clone(),
            meta,
            existed,
        });
    }
    let mut written: Vec<&ArchivedSnapshot> = Vec::new();
    for snapshot in &snapshots {
        if snapshot.meta.archived == archived {
            continue;
        }
        let mut next = snapshot.meta.clone();
        next.archived = archived;
        written.push(snapshot);
        if let Err(err) = save(Path::new(&snapshot.path), next) {
            let rollback = rollback_archived_meta_snapshots(&written, save);
            return Err(LineageArchiveError::Archive {
                source: err,
                rollback,
            });
        }
    }
    Ok(())
}

/// Restore every written snapshot, newest first (Go:
/// `rollbackArchivedMetaSnapshots`). Newly created sidecars are
/// removed; pre-existing ones get their old meta back. Errors are
/// collected Go-style (errors.Join: newline-joined).
fn rollback_archived_meta_snapshots(
    snapshots: &[&ArchivedSnapshot],
    save: &mut dyn FnMut(&Path, BranchMeta) -> Result<(), BranchMetaError>,
) -> Option<String> {
    let mut joined: Vec<String> = Vec::new();
    for snapshot in snapshots.iter().rev() {
        if !snapshot.existed {
            match std::fs::remove_file(naming::session_meta(Path::new(&snapshot.path))) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => joined.push(format!("remove new sidecar for {}: {err}", snapshot.path)),
            }
        } else if let Err(err) = save(Path::new(&snapshot.path), snapshot.meta.clone()) {
            joined.push(format!("restore sidecar for {}: {err}", snapshot.path));
        }
    }
    (!joined.is_empty()).then(|| joined.join("\n"))
}

/// Lineage of `session_path`: the path itself plus every listed
/// sibling recovery branch rooted at the same recovery root, minus
/// `except` (Go: `recoveryArchiveLineage`). Paths are
/// absolute and sorted.
fn recovery_archive_lineage(
    session_path: &str,
    except: &[&str],
) -> Result<Vec<String>, LineageArchiveError> {
    let abs = go_abs(session_path.trim());
    let mut excluded: HashSet<String> = HashSet::new();
    for path in except {
        let canonical = canonical_session_path(path);
        if !canonical.is_empty() {
            excluded.insert(canonical);
        }
    }
    let abs_text = abs.to_string_lossy().into_owned();
    if excluded.contains(&canonical_session_path(&abs_text)) {
        return Ok(Vec::new());
    }
    let listed = list_session_order(&go_dir(&abs))?;
    if let Some(joined) = unreadable_sidecar_err(&listed) {
        let base = abs
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        return Err(LineageArchiveError::ListLineage { base, joined });
    }
    let by_id: HashMap<String, SessionOrderInfo> = listed
        .iter()
        .map(|info| (branch_id(&info.path), info.clone()))
        .collect();
    let Some(target) = by_id.get(&branch_id(&abs_text)).cloned() else {
        return Ok(vec![abs_text]);
    };
    let root = recovery_archive_root(&target, &by_id);
    let mut paths = vec![abs_text.clone()];
    for info in &listed {
        if info.path == abs_text
            || !info.recovered
            || recovery_archive_root(info, &by_id) != root
            || (!excluded.is_empty() && excluded.contains(&canonical_session_path(&info.path)))
        {
            continue;
        }
        paths.push(info.path.clone());
    }
    paths.sort();
    Ok(paths)
}

/// Refuse the transaction when any listed sibling's meta is unreadable
/// (Go: `unreadableSidecarErr`): a zero `recovered` column
/// is indistinguishable from an unreadable sidecar, so the whole
/// transaction is refused.
fn unreadable_sidecar_err(listed: &[SessionOrderInfo]) -> Option<String> {
    let mut joined: Vec<String> = Vec::new();
    for info in listed {
        if let Err(err) = load_branch_meta(Path::new(&info.path)) {
            joined.push(err.to_string());
        }
    }
    (!joined.is_empty()).then(|| joined.join("\n"))
}

/// Recovery root of a listed branch (Go: `recoveryArchiveRoot`):
/// a stamped `recovery_root_id` wins; otherwise walk the parent
/// chain through listed siblings, capped at
/// `MAX_RECOVERY_ARCHIVE_LINEAGE_WALK` hops.
fn recovery_archive_root(
    info: &SessionOrderInfo,
    by_id: &HashMap<String, SessionOrderInfo>,
) -> String {
    let stamped = info.recovery_root_id.trim();
    if !stamped.is_empty() {
        return stamped.to_string();
    }
    let mut id = branch_id(&info.path);
    for _ in 0..MAX_RECOVERY_ARCHIVE_LINEAGE_WALK {
        let Some(current) = by_id.get(&id) else {
            return id;
        };
        if !current.recovered {
            return id;
        }
        let parent = current.parent_id.trim();
        if parent.is_empty() || parent == id {
            return id;
        }
        id = parent.to_string();
    }
    id
}

/// Go `filepath.Abs`: a relative path is joined onto the current
/// directory; no symlink resolution.
fn go_abs(path: &str) -> PathBuf {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(candidate),
            Err(_) => candidate.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::session::RecoveryBranchOptions;
    use crate::sessionstore::session::Session;
    use reasonix_contract::provider::{Message, Role};

    fn tempdir() -> tempfile::TempDir {
        crate::sessionstore::eventlog::test_support::tempdir()
    }

    fn user(content: &str) -> Message {
        Message {
            role: Role::USER,
            content: content.to_string(),
            ..Message::default()
        }
    }

    fn assistant(content: &str) -> Message {
        Message {
            role: Role::ASSISTANT,
            content: content.to_string(),
            ..Message::default()
        }
    }

    /// Go store.RemoveSessionArtifacts, best-effort form: the
    /// transcript and every sidecar file, then every sidecar directory.
    fn remove_session_artifacts(session_path: &Path) {
        let mut paths = vec![session_path.to_path_buf()];
        paths.extend(naming::session_sidecar_files(session_path));
        for path in paths {
            let _ = std::fs::remove_file(&path);
        }
        for dir in naming::session_sidecar_dirs(session_path) {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Go recovery_archive_test.go `seedRecoveryArchiveLineage`:
    /// a root whose artifacts are then removed, plus three chained
    /// recovery branches sharing its recovery root.
    fn seed_recovery_archive_lineage(dir: &Path) -> Vec<String> {
        let root = dir.join("20260803-140947-legacy.jsonl");
        let root_text = root.to_string_lossy().into_owned();
        let base = Session::new("sys");
        base.add(user("legacy prompt"));
        base.save(&root_text).expect("save root");
        let mut parent = root_text;
        let mut paths: Vec<String> = Vec::new();
        for i in 0u8..3 {
            let letter = (b'a' + i) as char;
            let copy = Session::new("sys");
            copy.add(user("legacy prompt"));
            copy.add(assistant(&letter.to_string()));
            let info = copy
                .save_recovery_branch(RecoveryBranchOptions {
                    original_path: parent.clone(),
                    ..RecoveryBranchOptions::default()
                })
                .expect("save recovery branch");
            paths.push(info.path.to_string_lossy().into_owned());
            parent = info.path.to_string_lossy().into_owned();
        }
        remove_session_artifacts(&root);
        paths
    }

    #[test]
    fn set_session_lineage_archived_rolls_back_partial_writes() {
        let dir = tempdir();
        let paths = seed_recovery_archive_lineage(dir.path());
        assert_eq!(paths.len(), 3);
        let fail_at = paths[1].clone();
        let mut failed = false;
        let mut save = |path: &Path, meta: BranchMeta| -> Result<(), BranchMetaError> {
            if path.to_string_lossy() == fail_at && !failed {
                failed = true;
                return Err(BranchMetaError::Io(io::Error::other(
                    "injected archive write failure",
                )));
            }
            let mut meta = meta;
            save_branch_meta_preserve_updated_locked(path, &mut meta)
        };
        let err = set_session_lineage_archived_with(&paths[0], true, &[], &mut save)
            .expect_err("archive must fail");
        assert!(
            err.to_string().contains("archive session lineage:"),
            "unexpected error: {err}"
        );
        for path in &paths {
            let meta_path = naming::session_meta(Path::new(path));
            assert!(meta_path.exists(), "sidecar for {path} must survive");
            let meta = load_branch_meta(Path::new(path))
                .expect("load meta")
                .expect("meta exists");
            assert!(!meta.archived, "rollback must restore {path}");
        }
    }

    #[test]
    fn set_session_lineage_archived_leaves_another_lineage_untouched() {
        let dir = tempdir();
        let paths = seed_recovery_archive_lineage(dir.path());
        let sessions_dir = go_dir(Path::new(&paths[0]));
        let other_root = sessions_dir.join("20260803-150000-other.jsonl");
        let other_root_text = other_root.to_string_lossy().into_owned();
        let base = Session::new("sys");
        base.add(user("other prompt"));
        base.save(&other_root_text).expect("save other root");
        let other = Session::new("sys");
        other.add(user("other prompt"));
        other.add(assistant("other reply"));
        let other_info = other
            .save_recovery_branch(RecoveryBranchOptions {
                original_path: other_root_text.clone(),
                ..RecoveryBranchOptions::default()
            })
            .expect("save other branch");
        remove_session_artifacts(&other_root);

        set_session_lineage_archived(&paths[0], true, &[]).expect("archive lineage");

        for path in &paths {
            let meta = load_branch_meta(Path::new(path))
                .expect("load meta")
                .expect("meta exists");
            assert!(meta.archived, "lineage sibling {path} must be archived");
        }
        let unrelated = load_branch_meta(&other_info.path)
            .expect("load unrelated meta")
            .expect("meta exists");
        assert!(!unrelated.archived, "another lineage must stay untouched");
    }
}
