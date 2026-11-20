//! Recovery-branch garbage collection (Go: recovery_gc.go).
//!
//! Decides when a recorded recovery branch is safe to reclaim: the
//! coverage predicate, the parent-guard acquisition that proves
//! coverage under a lock, the idle tracking behind the grace period,
//! and the trash entry points that move a provably covered branch
//! into the Desktop-recoverable trash layout.
//!
//! Contract: docs/contracts/session-format.md SS15.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, FixedOffset};

use super::branch::{BranchMeta, load_branch_meta};
use super::eventlog::writer_id::digest_string;
use super::recovery_lineage::{
    recovery_branch_redundant, recovery_branch_successor, try_acquire_recovery_coverage_guard,
};
use super::recovery_trash::{
    finish_recovery_trash_stage, prepare_recovery_trash_stage, reserve_recovery_trash_stage,
};
use super::session::storage_eq::{
    messages_have_prefix, messages_have_prefix_with_compatible_system,
};
use super::session::{digest_session_messages, is_visible_session, load_session};
use super::session_lease::{SessionLeaseError, session_lease_ownership};
use super::session_removal::{SessionRemovalGuard, try_acquire_session_removal_guard};

/// Grace period a recovery branch must stay idle before the reclaim
/// scan may collect it (Go: `RecoveryGCGracePeriod`).
#[allow(dead_code)] // wired by the reclaim scan (SESS-039)
pub const RECOVERY_GC_GRACE_PERIOD: Duration = Duration::from_secs(24 * 60 * 60);

/// Shorter grace applied to scans triggered at startup (Go:
/// `RecoveryGCStartupGracePeriod`).
#[allow(dead_code)] // wired by the reclaim scan (SESS-039)
pub const RECOVERY_GC_STARTUP_GRACE_PERIOD: Duration = Duration::from_secs(15 * 60);

/// Directory holding trashed recovery sessions inside a sessions
/// directory (Go: `recoveryTrashDir`).
#[allow(dead_code)] // wired by the trash pipeline (SESS-040)
pub const RECOVERY_TRASH_DIR: &str = ".trash";

/// Name of the per-item trash metadata file (Go:
/// `recoveryTrashMetaFile`).
#[allow(dead_code)] // wired by the trash pipeline (SESS-040)
pub const RECOVERY_TRASH_META_FILE: &str = ".trash-meta.json";

/// Prefix every recovery-trash operation carries in logs (Go:
/// `recoveryTrashOperationPrefix`).
#[allow(dead_code)] // wired by the trash pipeline (SESS-040)
pub const RECOVERY_TRASH_OPERATION_PREFIX: &str = "recovery-trash:";

/// Pending-move marker written before the artifacts move (Go:
/// `recoveryTrashPendingFile`).
#[allow(dead_code)] // wired by the trash pipeline (SESS-040)
pub const RECOVERY_TRASH_PENDING_FILE: &str = ".recovery-trash-pending.json";

/// Staging directory prefix for in-progress trashes (Go:
/// `recoveryTrashStagingPrefix`).
#[allow(dead_code)] // wired by the trash pipeline (SESS-040)
pub const RECOVERY_TRASH_STAGING_PREFIX: &str = ".recovery-trash-staging-";

/// Sentinel error text when a guard proves a branch uncovered (Go:
/// `ErrRecoveryBranchNotCovered`).
#[allow(dead_code)] // wired by guard acquisition (SESS-043)
pub const ERR_RECOVERY_BRANCH_NOT_COVERED: &str = "recovery branch is not covered by its parent";

/// Sentinel error text when a branch is still inside its grace period
/// (Go: `ErrRecoveryBranchNotIdle`).
#[allow(dead_code)] // wired by guard acquisition (SESS-043)
pub const ERR_RECOVERY_BRANCH_NOT_IDLE: &str =
    "recovery branch is still inside its safety grace period";

/// Whether the recovery branch at `path` is a recorded recovery
/// branch whose transcript is fully covered by its recorded parent
/// (Go: `RecoveryBranchCoveredByParent`).
///
/// Refuses to answer when the sidecar is unreadable or absent: only a
/// provably covered branch counts as covered.
pub fn recovery_branch_covered_by_parent(path: &Path, parent_dir: &Path) -> bool {
    let Ok(Some(meta)) = load_branch_meta(path) else {
        return false;
    };
    if !meta.recovered || meta.recovery_digest.trim().is_empty() {
        return false;
    }
    recovery_branch_covered_by_parent_meta(path, parent_dir, &meta)
}

/// Meta-gated coverage core (Go: `recoveryBranchCoveredByParent`).
pub(crate) fn recovery_branch_covered_by_parent_meta(
    path: &Path,
    parent_dir: &Path,
    meta: &BranchMeta,
) -> bool {
    let parent_id = meta.parent_id.trim();
    if parent_id.is_empty() {
        return false;
    }
    // Continued on (or undigestable): this is someone's conversation
    // now, so coverage cannot be proven.
    let Ok(branch_session) = load_session(&path.to_string_lossy()) else {
        return false;
    };
    let branch_msgs = branch_session.snapshot();
    let Ok(branch_digest) = digest_session_messages(&branch_msgs) else {
        return false;
    };
    if digest_string(&branch_digest) != meta.recovery_digest.trim() {
        return false;
    }
    let parent_dir = trimmed_dir_or(path, parent_dir);
    let parent_path = parent_dir.join(format!("{parent_id}.jsonl"));
    if parent_path == path || !is_visible_session(&parent_path.to_string_lossy()) {
        return false;
    }
    let Ok(parent_session) = load_session(&parent_path.to_string_lossy()) else {
        return false;
    };
    let parent_msgs = parent_session.snapshot();
    let Ok(parent_digest) = digest_session_messages(&parent_msgs) else {
        return false;
    };
    parent_digest == branch_digest
        || messages_have_prefix(&parent_msgs, &branch_msgs)
        || messages_have_prefix_with_compatible_system(&parent_msgs, &branch_msgs)
}

/// Parent directory of `path`, `.` for bare names (mirrors
/// the private Go `file_dir` helpers).
pub(crate) fn go_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Directory override with Go's blank fallback: a blank `dir`
/// falls back to the branch's own directory (Go: `dir :=
/// TrimSpace(dir); if dir == "" { dir = filepath.Dir(path) }`).
pub(crate) fn trimmed_dir_or(path: &Path, dir: &Path) -> PathBuf {
    let dir_text = dir.to_string_lossy();
    let trimmed = dir_text.trim();
    if trimmed.is_empty() {
        go_dir(path)
    } else {
        PathBuf::from(trimmed)
    }
}

/// Errors surfaced by the recovery reclaim and trash entry points
/// (Go: the `ErrRecoveryBranchNotCovered` and
/// `ErrRecoveryBranchNotIdle` sentinels plus the lease failures
/// underneath the guard acquisitions).
#[derive(Debug, thiserror::Error)]
pub enum RecoveryGcError {
    /// Coverage could not be proven (Go:
    /// `ErrRecoveryBranchNotCovered`).
    #[error("{}", ERR_RECOVERY_BRANCH_NOT_COVERED)]
    NotCovered,
    /// The branch is still inside its safety grace period (Go:
    /// `ErrRecoveryBranchNotIdle`).
    #[error("{}", ERR_RECOVERY_BRANCH_NOT_IDLE)]
    NotIdle,
    /// A lease or lock was held by another writer (Go: the wrapped
    /// `*SessionLeaseError`).
    #[error(transparent)]
    Lease(#[from] SessionLeaseError),
    /// Any other filesystem failure.
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl RecoveryGcError {
    /// Go: `errors.Is(err, ErrRecoveryBranchNotCovered)`.
    pub fn is_not_covered(&self) -> bool {
        matches!(self, RecoveryGcError::NotCovered)
    }

    /// Go: `errors.Is(err, ErrRecoveryBranchNotIdle)`.
    pub fn is_not_idle(&self) -> bool {
        matches!(self, RecoveryGcError::NotIdle)
    }

    /// Go: `errors.Is(err, ErrSessionLeaseHeld)` on the wrapped
    /// lease failure.
    pub fn is_lease_held(&self) -> bool {
        matches!(self, RecoveryGcError::Lease(err) if err.is_held())
    }
}

/// Whether any runtime currently holds the session lease at `path`
/// (Go: `SessionLeaseHeld`).
pub fn session_lease_held(path: &str) -> bool {
    if path.trim().is_empty() {
        return false;
    }
    session_lease_ownership(path).is_err()
}

/// Take the removal guard of a recovery branch's recorded parent
/// after proving the branch is covered (Go:
/// `TryAcquireRecoveryParentGuard`).
pub fn try_acquire_recovery_parent_guard(
    path: &Path,
    parent_dir: &Path,
) -> Result<SessionRemovalGuard, RecoveryGcError> {
    let Ok(Some(meta)) = load_branch_meta(path) else {
        return Err(RecoveryGcError::NotCovered);
    };
    if !meta.recovered || meta.recovery_digest.trim().is_empty() {
        return Err(RecoveryGcError::NotCovered);
    }
    let parent_id = meta.parent_id.trim();
    if parent_id.is_empty() {
        return Err(RecoveryGcError::NotCovered);
    }
    let parent_dir = trimmed_dir_or(path, parent_dir);
    let parent_path = parent_dir.join(format!("{parent_id}.jsonl"));
    if parent_path == path || !is_visible_session(&parent_path.to_string_lossy()) {
        return Err(RecoveryGcError::NotCovered);
    }
    let guard = try_acquire_session_removal_guard(&parent_path.to_string_lossy())
        .map_err(RecoveryGcError::Lease)?;
    if !recovery_branch_covered_by_parent_meta(path, &parent_dir, &meta) {
        guard.release();
        return Err(RecoveryGcError::NotCovered);
    }
    Ok(guard)
}

/// Recovery branches eligible for reclaim (Go:
/// `ReclaimableRecoveryBranches`): recorded, idle past `grace`,
/// lease-free, and provably covered by their parent or a successor.
/// Sorted by path.
pub fn reclaimable_recovery_branches(
    dir: &Path,
    now: &DateTime<FixedOffset>,
    grace: Duration,
) -> io::Result<Vec<PathBuf>> {
    let dir_text = dir.to_string_lossy();
    let trimmed = dir_text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let dir = Path::new(trimmed);
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let name = name.to_string_lossy();
        if is_dir || !name.ends_with(".jsonl") || name.ends_with(".events.jsonl") {
            continue;
        }
        let path = dir.join(name.as_ref());
        if !is_visible_session(&path.to_string_lossy()) {
            continue;
        }
        let Ok(Some(meta)) = load_branch_meta(&path) else {
            continue;
        };
        if !meta.recovered || meta.recovery_digest.trim().is_empty() {
            continue;
        }
        if meta.parent_id.trim().is_empty() {
            continue;
        }
        if !recovery_branch_idle(&path, &meta, now, grace) {
            continue;
        }
        if session_lease_held(&path.to_string_lossy()) {
            continue;
        }
        if !recovery_branch_covered_by_parent_meta(&path, dir, &meta)
            && recovery_branch_successor(&path, dir, &meta).is_none()
        {
            continue;
        }
        out.push(path);
    }
    out.sort();
    Ok(out)
}

/// Trash a covered recovery branch into the Desktop-recoverable
/// layout without checking idleness (Go: `TrashCoveredRecoveryBranch`).
pub fn trash_covered_recovery_branch(
    path: &Path,
    parent_dir: &Path,
) -> Result<(), RecoveryGcError> {
    trash_covered_recovery_branch_inner(path, parent_dir, false)
}

/// Trash a reclaimable recovery branch, enforcing the idle grace at
/// the final guard (Go: `TrashReclaimableRecoveryBranch`).
pub fn trash_reclaimable_recovery_branch(
    path: &Path,
    parent_dir: &Path,
) -> Result<(), RecoveryGcError> {
    trash_covered_recovery_branch_inner(path, parent_dir, true)
}

fn trash_covered_recovery_branch_inner(
    path: &Path,
    parent_dir: &Path,
    require_idle: bool,
) -> Result<(), RecoveryGcError> {
    // Go filepath.Clean(TrimSpace(.)): callers pass canonical paths,
    // so trimming suffices; a blank path is "." either way.
    let path_text = path.to_string_lossy().trim().to_string();
    let parent_text = parent_dir.to_string_lossy().trim().to_string();
    let path_text = if path_text.is_empty() {
        ".".to_string()
    } else {
        path_text
    };
    let parent_text = if parent_text.is_empty() {
        ".".to_string()
    } else {
        parent_text
    };
    if path_text == "."
        || parent_text == "."
        || go_dir(Path::new(&path_text)) != Path::new(&parent_text)
    {
        return Err(RecoveryGcError::Io(io::Error::other(
            "recovery branch must be a direct child of its session directory",
        )));
    }
    let key = Path::new(&path_text)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !key.ends_with(".jsonl") || key.ends_with(".events.jsonl") {
        return Err(RecoveryGcError::Io(io::Error::other(
            "invalid recovery session path",
        )));
    }
    let coverage_guard =
        try_acquire_recovery_coverage_guard(Path::new(&path_text), Path::new(&parent_text))?;
    let mut branch_guard = None;
    let result = (|| -> Result<(), RecoveryGcError> {
        branch_guard =
            Some(try_acquire_session_removal_guard(&path_text).map_err(RecoveryGcError::Lease)?);
        if require_idle {
            let Ok(Some(meta)) = load_branch_meta(Path::new(&path_text)) else {
                return Err(RecoveryGcError::NotIdle);
            };
            if !recovery_branch_idle(
                Path::new(&path_text),
                &meta,
                &chrono::Utc::now().fixed_offset(),
                RECOVERY_GC_GRACE_PERIOD,
            ) {
                return Err(RecoveryGcError::NotIdle);
            }
        }
        if !recovery_branch_redundant(Path::new(&path_text), Path::new(&parent_text)) {
            return Err(RecoveryGcError::NotCovered);
        }
        let stage_dir =
            reserve_recovery_trash_stage(Path::new(&parent_text)).map_err(RecoveryGcError::Io)?;
        prepare_recovery_trash_stage(Path::new(&path_text), &key, &stage_dir)
            .map_err(RecoveryGcError::Io)?;
        let guard = branch_guard.take().expect("branch guard held");
        finish_recovery_trash_stage(
            Path::new(&parent_text),
            Path::new(&path_text),
            &key,
            &stage_dir,
            guard,
        )
        .map_err(RecoveryGcError::Io)
    })();
    if let Some(guard) = branch_guard {
        guard.release();
    }
    coverage_guard.release();
    result
}

/// Whether the branch has been idle for `grace` (Go:
/// `recoveryBranchIdle`): the sidecar's `updated_at`, or the file's
/// modification time when the sidecar carries no timestamp.
fn recovery_branch_idle(
    path: &Path,
    meta: &BranchMeta,
    now: &DateTime<FixedOffset>,
    grace: Duration,
) -> bool {
    let idle_since = if meta.updated_at.is_zero() {
        let Ok(info) = std::fs::metadata(path) else {
            return false;
        };
        let Ok(modified) = info.modified() else {
            return false;
        };
        chrono::DateTime::<chrono::Utc>::from(modified).fixed_offset()
    } else {
        meta.updated_at.get()
    };
    let Ok(grace) = chrono::Duration::from_std(grace) else {
        return false;
    };
    *now - idle_since >= grace
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::branch::{
        branch_id, save_branch_meta, save_branch_meta_preserve_updated,
    };
    use crate::sessionstore::cleanup_pending::{CleanupPendingInfo, reconcile_cleanup_pending};
    use crate::sessionstore::eventlog::record::CreatedAt;
    use crate::sessionstore::recovery_trash::{
        prepare_recovery_trash_entry, prepare_recovery_trash_stage,
        reserve_recovery_trash_item_dir, reserve_recovery_trash_stage,
    };
    use crate::sessionstore::session::{RecoveryBranchOptions, Session, is_cleanup_pending};
    use crate::sessionstore::session_lease::try_acquire_session_lease;
    use crate::sessionstore::session_lock::{
        SessionFileLockError, lock_session_file, try_take_session_lock_file,
    };
    use crate::sessionstore::session_order::mark_cleanup_pending;
    use crate::store::naming;
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

    fn recovery_meta(path: &Path, root: &Path, digest: [u8; 32]) -> BranchMeta {
        BranchMeta {
            id: branch_id(&path.to_string_lossy()),
            parent_id: branch_id(&root.to_string_lossy()),
            recovered: true,
            recovery_digest: digest_string(&digest),
            recovery_depth: 1,
            ..BranchMeta::default()
        }
    }

    #[test]
    fn parent_coverage_holds_when_parent_keeps_branch_as_prefix() {
        let dir = tempdir();
        let root = dir.path().join("session.jsonl");
        let parent = Session::new("sys");
        parent.add(user("first"));
        parent.add(assistant("reply"));
        parent.add(user("more"));
        parent.save(&root.to_string_lossy()).expect("save parent");

        let branch = dir.path().join("session-recovery-1.jsonl");
        let live = Session::new("sys");
        live.add(user("first"));
        live.add(assistant("reply"));
        live.save(&branch.to_string_lossy()).expect("save branch");
        let digest = digest_session_messages(&live.snapshot()).expect("digest");
        save_branch_meta(&branch, recovery_meta(&branch, &root, digest)).expect("save meta");

        assert!(recovery_branch_covered_by_parent(&branch, dir.path()));
    }

    #[test]
    fn parent_coverage_holds_for_identical_transcripts() {
        let dir = tempdir();
        let root = dir.path().join("session.jsonl");
        let parent = Session::new("sys");
        parent.add(user("first"));
        parent.add(assistant("reply"));
        parent.save(&root.to_string_lossy()).expect("save parent");

        let branch = dir.path().join("session-recovery-2.jsonl");
        let live = Session::new("sys");
        live.add(user("first"));
        live.add(assistant("reply"));
        live.save(&branch.to_string_lossy()).expect("save branch");
        let digest = digest_session_messages(&live.snapshot()).expect("digest");
        save_branch_meta(&branch, recovery_meta(&branch, &root, digest)).expect("save meta");

        assert!(recovery_branch_covered_by_parent(&branch, dir.path()));
    }

    #[test]
    fn parent_coverage_refuses_stale_digest() {
        let dir = tempdir();
        let root = dir.path().join("session.jsonl");
        let parent = Session::new("sys");
        parent.add(user("first"));
        parent.add(assistant("reply"));
        parent.save(&root.to_string_lossy()).expect("save parent");

        let branch = dir.path().join("session-recovery-3.jsonl");
        let live = Session::new("sys");
        live.add(user("first"));
        live.add(assistant("reply"));
        live.save(&branch.to_string_lossy()).expect("save branch");
        let mut stale = [0u8; 32];
        stale[..8].copy_from_slice(b"stale!!!");
        save_branch_meta(&branch, recovery_meta(&branch, &root, stale)).expect("save meta");

        assert!(!recovery_branch_covered_by_parent(&branch, dir.path()));
    }

    #[test]
    fn parent_coverage_refuses_diverged_parent() {
        let dir = tempdir();
        let root = dir.path().join("session.jsonl");
        let parent = Session::new("sys");
        parent.add(user("first"));
        parent.save(&root.to_string_lossy()).expect("save parent");

        let branch = dir.path().join("session-recovery-4.jsonl");
        let live = Session::new("sys");
        live.add(user("first"));
        live.add(assistant("reply"));
        live.add(user("extra turn"));
        live.save(&branch.to_string_lossy()).expect("save branch");
        let digest = digest_session_messages(&live.snapshot()).expect("digest");
        save_branch_meta(&branch, recovery_meta(&branch, &root, digest)).expect("save meta");

        assert!(!recovery_branch_covered_by_parent(&branch, dir.path()));
    }

    #[test]
    fn parent_coverage_refuses_self_parent_and_plain_sessions() {
        let dir = tempdir();
        let root = dir.path().join("session.jsonl");
        let parent = Session::new("sys");
        parent.add(user("first"));
        parent.add(assistant("reply"));
        parent.save(&root.to_string_lossy()).expect("save parent");

        let branch = dir.path().join("session-recovery-5.jsonl");
        let live = Session::new("sys");
        live.add(user("first"));
        live.add(assistant("reply"));
        live.save(&branch.to_string_lossy()).expect("save branch");
        let digest = digest_session_messages(&live.snapshot()).expect("digest");

        // A branch cannot be its own parent.
        let mut self_parent = recovery_meta(&branch, &root, digest);
        self_parent.parent_id = branch_id(&branch.to_string_lossy());
        save_branch_meta(&branch, self_parent).expect("save meta");
        assert!(!recovery_branch_covered_by_parent(&branch, dir.path()));

        // A plain session is not a recovery branch.
        let mut not_recovered = recovery_meta(&branch, &root, digest);
        not_recovered.recovered = false;
        save_branch_meta(&branch, not_recovered).expect("save meta");
        assert!(!recovery_branch_covered_by_parent(&branch, dir.path()));
    }

    // ---- Go recovery_gc_test.go ----

    fn fork_recovery_branch(dir: &Path, name: &str) -> (PathBuf, PathBuf, Vec<Message>) {
        let parent_path = dir.join(format!("{name}.jsonl"));
        let disk = Session::new("sys");
        disk.add(user("first"));
        disk.add(assistant("one"));
        disk.add(user(&format!("disk {name}")));
        disk.save(&parent_path.to_string_lossy())
            .expect("save parent");

        let stale = Session::new("sys");
        stale.add(user("first"));
        stale.add(assistant("one"));
        stale.add(user(&format!("local {name}")));
        let info = stale
            .save_recovery_branch(RecoveryBranchOptions {
                original_path: parent_path.to_string_lossy().into_owned(),
                ..RecoveryBranchOptions::default()
            })
            .expect("save recovery branch");
        (parent_path, info.path, stale.snapshot())
    }

    fn cover_branch_in_parent(parent_path: &Path, branch_msgs: &[Message]) {
        let merged = load_session(&parent_path.to_string_lossy()).expect("load parent");
        merged.replace(branch_msgs.to_vec());
        merged.add(assistant("answered after recovery"));
        merged
            .save_rewrite(&parent_path.to_string_lossy())
            .expect("rewrite parent");
    }

    fn age_recovery_branch_for_gc(path: &Path) {
        let mut meta = load_branch_meta(path)
            .expect("load branch meta")
            .expect("branch meta present");
        meta.updated_at = CreatedAt::from_datetime(
            (chrono::Utc::now() - chrono::Duration::seconds(2 * 24 * 3600)).fixed_offset(),
        );
        save_branch_meta_preserve_updated(path, meta).expect("age branch meta");
    }

    #[test]
    fn collects_only_covered_idle_forks() {
        let dir = tempdir();
        let later = (chrono::Utc::now() + chrono::Duration::hours(48)).fixed_offset();

        let (parent, covered, msgs) = fork_recovery_branch(dir.path(), "covered");
        cover_branch_in_parent(&parent, &msgs);

        let (_diverged_parent, diverged, _diverged_msgs) =
            fork_recovery_branch(dir.path(), "diverged");
        assert!(diverged.exists());

        let (parent_c, continued, msgs_c) = fork_recovery_branch(dir.path(), "continued");
        cover_branch_in_parent(&parent_c, &msgs_c);
        let sess = load_session(&continued.to_string_lossy()).expect("load branch");
        sess.add(assistant("user kept chatting here"));
        sess.save(&continued.to_string_lossy())
            .expect("continue branch");

        let got = reclaimable_recovery_branches(dir.path(), &later, RECOVERY_GC_GRACE_PERIOD)
            .expect("reclaimable listing");
        assert_eq!(got, vec![covered]);
    }

    #[test]
    fn respects_grace_lease_and_missing_parent() {
        let dir = tempdir();
        let (parent, branch, msgs) = fork_recovery_branch(dir.path(), "guarded");
        cover_branch_in_parent(&parent, &msgs);
        let now = chrono::Utc::now().fixed_offset();
        let later = (chrono::Utc::now() + chrono::Duration::hours(48)).fixed_offset();

        // A fresh fork is still inside its grace period.
        assert!(
            reclaimable_recovery_branches(dir.path(), &now, RECOVERY_GC_GRACE_PERIOD)
                .expect("fresh listing")
                .is_empty()
        );

        // A held session lease blocks the reclaim even past the grace.
        let lease = try_acquire_session_lease(&branch.to_string_lossy()).expect("session lease");
        assert!(session_lease_held(&branch.to_string_lossy()));
        assert!(
            reclaimable_recovery_branches(dir.path(), &later, RECOVERY_GC_GRACE_PERIOD)
                .expect("leased listing")
                .is_empty()
        );
        lease.release();
        assert!(!session_lease_held(&branch.to_string_lossy()));
        assert_eq!(
            reclaimable_recovery_branches(dir.path(), &later, RECOVERY_GC_GRACE_PERIOD)
                .expect("released listing"),
            vec![branch.clone()]
        );

        // Without the parent transcript, coverage cannot be proven.
        std::fs::remove_file(&parent).expect("remove parent");
        for sidecar in naming::session_sidecar_files(&parent) {
            let _ = std::fs::remove_file(&sidecar);
        }
        assert!(
            reclaimable_recovery_branches(dir.path(), &later, RECOVERY_GC_GRACE_PERIOD)
                .expect("missing-parent listing")
                .is_empty()
        );
    }

    #[test]
    fn covered_by_parent_reads_actual_content() {
        let dir = tempdir();
        let (_parent, diverged, _diverged_msgs) =
            fork_recovery_branch(dir.path(), "diverged-proof");
        assert!(!recovery_branch_covered_by_parent(&diverged, dir.path()));

        let (parent, covered, msgs) = fork_recovery_branch(dir.path(), "covered-proof");
        cover_branch_in_parent(&parent, &msgs);
        assert!(recovery_branch_covered_by_parent(&covered, dir.path()));

        // A branch that outgrew its sidecar digest is not covered.
        let stale_meta = load_branch_meta(&covered)
            .expect("load branch meta")
            .expect("branch meta present");
        let sess = load_session(&covered.to_string_lossy()).expect("load branch");
        sess.add(assistant("continued after sidecar snapshot"));
        sess.save(&covered.to_string_lossy())
            .expect("continue branch");
        save_branch_meta_preserve_updated(&covered, stale_meta).expect("restore stale meta");
        assert!(!recovery_branch_covered_by_parent(&covered, dir.path()));

        // Coverage follows the parent transcript, not the fork record.
        let (parent_m, missing, msgs_m) = fork_recovery_branch(dir.path(), "missing-proof");
        cover_branch_in_parent(&parent_m, &msgs_m);
        assert!(recovery_branch_covered_by_parent(&missing, dir.path()));
        std::fs::remove_file(&parent_m).expect("remove parent");
        for sidecar in naming::session_sidecar_files(&parent_m) {
            let _ = std::fs::remove_file(&sidecar);
        }
        assert!(!recovery_branch_covered_by_parent(&missing, dir.path()));
    }

    #[test]
    fn parent_guard_blocks_rewind_after_validation() {
        let dir = tempdir();
        let (parent, branch, msgs) = fork_recovery_branch(dir.path(), "rewind-race");
        cover_branch_in_parent(&parent, &msgs);

        let guard = try_acquire_recovery_parent_guard(&branch, dir.path()).expect("parent guard");
        match try_take_session_lock_file(&naming::session_lock_file(&parent)) {
            Ok(lock) => {
                lock.unlock();
                guard.release();
                panic!("the parent guard must hold the session lock");
            }
            Err(SessionFileLockError::Held) => {}
            Err(SessionFileLockError::Io(err)) => {
                guard.release();
                panic!("unexpected session lock error: {err}");
            }
        }
        guard.release();

        // A rewind after validation breaks the recorded coverage.
        let sess = load_session(&parent.to_string_lossy()).expect("load parent");
        let mut truncated = sess.snapshot();
        truncated.truncate(1);
        sess.replace(truncated);
        sess.save_rewrite(&parent.to_string_lossy())
            .expect("rewrite parent");
        assert!(!recovery_branch_covered_by_parent(&branch, dir.path()));
    }

    #[test]
    fn parent_guard_refuses_in_flight_parent_rewrite() {
        let dir = tempdir();
        let (parent, branch, msgs) = fork_recovery_branch(dir.path(), "rewrite-busy");
        cover_branch_in_parent(&parent, &msgs);

        let unlock = match lock_session_file(&parent) {
            Ok(lock) => lock,
            Err(SessionFileLockError::Held) => panic!("parent session already locked"),
            Err(SessionFileLockError::Io(err)) => panic!("lock parent failed: {err}"),
        };
        let err = try_acquire_recovery_parent_guard(&branch, dir.path())
            .expect_err("parent guard must refuse an in-flight rewrite");
        assert!(err.is_lease_held(), "expected a held lease, got: {err}");
        drop(unlock);
    }

    #[test]
    fn trash_uses_recoverable_desktop_layout() {
        let dir = tempdir();
        let (parent, branch, msgs) = fork_recovery_branch(dir.path(), "trash-covered");
        cover_branch_in_parent(&parent, &msgs);
        age_recovery_branch_for_gc(&branch);

        trash_reclaimable_recovery_branch(&branch, dir.path()).expect("trash branch");

        let key = branch
            .file_name()
            .expect("branch name")
            .to_string_lossy()
            .into_owned();
        assert!(!branch.exists());
        assert!(!is_cleanup_pending(&branch.to_string_lossy()));
        let item_dir = dir.path().join(RECOVERY_TRASH_DIR).join(&key);
        assert!(item_dir.join(&key).exists(), "trashed transcript");
        assert!(
            item_dir
                .join(
                    naming::session_meta(&branch)
                        .file_name()
                        .expect("meta name")
                )
                .exists(),
            "trashed branch meta"
        );
        assert!(
            item_dir
                .join(
                    naming::session_event_log(&branch)
                        .file_name()
                        .expect("log name")
                )
                .exists(),
            "trashed event log"
        );
        assert!(
            item_dir.join(RECOVERY_TRASH_META_FILE).exists(),
            "trash meta"
        );
        assert!(parent.exists(), "parent survives the trash");
        assert!(!item_dir.join(RECOVERY_TRASH_PENDING_FILE).exists());
    }

    #[test]
    fn trash_enforces_grace_at_final_guard() {
        let dir = tempdir();
        let (_parent, branch, msgs) = fork_recovery_branch(dir.path(), "trash-fresh");
        cover_branch_in_parent(&_parent, &msgs);

        let err = trash_reclaimable_recovery_branch(&branch, dir.path())
            .expect_err("a fresh branch must not be trashable");
        assert!(err.is_not_idle(), "expected the grace refusal, got: {err}");
        assert!(branch.exists());
    }

    #[test]
    fn interrupted_stage_reconciles_before_publication() {
        let dir = tempdir();
        let (_parent, branch, _msgs) = fork_recovery_branch(dir.path(), "staged-crash");
        let key = branch
            .file_name()
            .expect("branch name")
            .to_string_lossy()
            .into_owned();

        let stage_dir = reserve_recovery_trash_stage(dir.path()).expect("reserve stage");
        prepare_recovery_trash_stage(&branch, &key, &stage_dir).expect("prepare stage");

        assert!(!is_cleanup_pending(&branch.to_string_lossy()));
        assert!(!branch.exists());
        assert!(!naming::is_session_transcript_name(
            stage_dir.file_name().expect("stage name"),
        ));
        assert!(stage_dir.join(&key).exists());
        assert!(stage_dir.join(RECOVERY_TRASH_PENDING_FILE).exists());
        assert!(!stage_dir.join(RECOVERY_TRASH_META_FILE).exists());

        let mut called = 0;
        let mut cb = |_item: CleanupPendingInfo| -> io::Result<()> {
            called += 1;
            Ok(())
        };
        reconcile_cleanup_pending(
            &dir.path().to_string_lossy(),
            Some(&mut cb as &mut dyn FnMut(CleanupPendingInfo) -> io::Result<()>),
        )
        .expect("reconcile the interrupted stage");
        assert_eq!(called, 0);

        assert!(!stage_dir.exists());
        let item_dir = dir.path().join(RECOVERY_TRASH_DIR).join(&key);
        assert!(item_dir.join(&key).exists());
        assert!(
            item_dir
                .join(
                    naming::session_meta(&branch)
                        .file_name()
                        .expect("meta name")
                )
                .exists()
        );
        assert!(
            item_dir
                .join(
                    naming::session_event_log(&branch)
                        .file_name()
                        .expect("log name")
                )
                .exists()
        );
        assert!(item_dir.join(RECOVERY_TRASH_META_FILE).exists());
        assert!(!item_dir.join(RECOVERY_TRASH_PENDING_FILE).exists());
    }

    #[test]
    fn finishes_recovery_trash_without_hard_delete_callback() {
        let dir = tempdir();
        let (_parent, branch, _msgs) = fork_recovery_branch(dir.path(), "trash-interrupted");
        let key = branch
            .file_name()
            .expect("branch name")
            .to_string_lossy()
            .into_owned();

        let (item_name, item_dir) =
            reserve_recovery_trash_item_dir(dir.path(), &key).expect("reserve item dir");
        prepare_recovery_trash_entry(&branch, &key, &item_dir).expect("prepare entry");
        mark_cleanup_pending(
            &branch.to_string_lossy(),
            &format!("{RECOVERY_TRASH_OPERATION_PREFIX}{item_name}"),
        )
        .expect("mark pending");

        let mut called = 0;
        let mut cb = |_item: CleanupPendingInfo| -> io::Result<()> {
            called += 1;
            Ok(())
        };
        reconcile_cleanup_pending(
            &dir.path().to_string_lossy(),
            Some(&mut cb as &mut dyn FnMut(CleanupPendingInfo) -> io::Result<()>),
        )
        .expect("reconcile the interrupted move");
        assert_eq!(called, 0);
        assert!(!is_cleanup_pending(&branch.to_string_lossy()));
        assert!(item_dir.join(RECOVERY_TRASH_META_FILE).exists());
    }
}
