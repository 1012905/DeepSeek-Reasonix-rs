//! Recovery-lineage redundancy predicates (Go: recovery_lineage.go).
//!
//! A recovery branch is redundant when some other session provably
//! holds every message the branch holds: either its recorded parent
//! covers it, or a successor in the same directory does. Both
//! predicates refuse to answer on unreadable or ambiguous input - a
//! branch only counts as redundant when coverage is proven.
//!
//! Contract: docs/contracts/session-format.md SS15.3.

use std::path::{Path, PathBuf};

use super::branch::{BranchMeta, load_branch_meta};
use super::eventlog::writer_id::digest_string;
use super::recovery_gc::{
    RecoveryGcError, recovery_branch_covered_by_parent_meta, trimmed_dir_or,
    try_acquire_recovery_parent_guard,
};
use super::session::storage_eq::{
    messages_have_prefix, messages_have_prefix_with_compatible_system,
};
use super::session::{digest_session_messages, is_visible_session, load_session};
use super::session_removal::{SessionRemovalGuard, try_acquire_session_removal_guard};

/// Whether some other session provably holds every message of the
/// recovery branch at `path` (Go: `recoveryBranchRedundant`).
pub fn recovery_branch_redundant(path: &Path, dir: &Path) -> bool {
    let Ok(Some(meta)) = load_branch_meta(path) else {
        return false;
    };
    if !meta.recovered || meta.recovery_digest.trim().is_empty() {
        return false;
    }
    if recovery_branch_covered_by_parent_meta(path, dir, &meta) {
        return true;
    }
    recovery_branch_successor(path, dir, &meta).is_some()
}

/// First successor in `dir` that provably covers the branch (Go:
/// `recoveryBranchSuccessor`). Lineage links are not required.
pub(crate) fn recovery_branch_successor(
    path: &Path,
    dir: &Path,
    meta: &BranchMeta,
) -> Option<PathBuf> {
    let dir = trimmed_dir_or(path, dir);
    let entries = std::fs::read_dir(&dir).ok()?;
    // Go's os.ReadDir returns entries sorted by name; mirror that so
    // the chosen successor is deterministic.
    let mut names: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    for name in names {
        if !name.ends_with(".jsonl") || name.ends_with(".events.jsonl") {
            continue;
        }
        let candidate = dir.join(&name);
        if candidate == path {
            continue;
        }
        let Ok(Some(candidate_meta)) = load_branch_meta(&candidate) else {
            continue;
        };
        if !candidate_meta.recovered {
            continue;
        }
        if recovery_branch_covered_by_successor(path, &candidate, meta) {
            return Some(candidate);
        }
    }
    None
}

/// Whether `successor` provably covers the branch (Go:
/// `recoveryBranchCoveredBySuccessor`).
pub(crate) fn recovery_branch_covered_by_successor(
    path: &Path,
    successor: &Path,
    meta: &BranchMeta,
) -> bool {
    if !is_visible_session(&successor.to_string_lossy()) {
        return false;
    }
    let Ok(session) = load_session(&path.to_string_lossy()) else {
        return false;
    };
    let msgs = session.snapshot();
    let Ok(digest) = digest_session_messages(&msgs) else {
        return false;
    };
    if digest_string(&digest) != meta.recovery_digest.trim() {
        return false;
    }
    let Ok(later_session) = load_session(&successor.to_string_lossy()) else {
        return false;
    };
    let later_msgs = later_session.snapshot();
    if !messages_have_prefix(&later_msgs, &msgs)
        && !messages_have_prefix_with_compatible_system(&later_msgs, &msgs)
    {
        return false;
    }
    // Two identical copies of one transcript cover each other; keeping
    // the later path stops both copies from proving each other
    // redundant and vanishing together (Go tie-break).
    later_msgs.len() > msgs.len() || path.to_string_lossy() < successor.to_string_lossy()
}

/// Acquire the removal guard of whichever session proves the branch
/// covered (Go: `tryAcquireRecoveryCoverageGuard`): the recorded
/// parent first, then a covering successor.
pub(crate) fn try_acquire_recovery_coverage_guard(
    path: &Path,
    dir: &Path,
) -> Result<SessionRemovalGuard, RecoveryGcError> {
    match try_acquire_recovery_parent_guard(path, dir) {
        Ok(guard) => Ok(guard),
        Err(RecoveryGcError::NotCovered) => try_acquire_recovery_successor_guard(path, dir),
        Err(err) => Err(err),
    }
}

/// Acquire the removal guard of the branch's covering successor (Go:
/// `tryAcquireRecoverySuccessorGuard`).
pub(crate) fn try_acquire_recovery_successor_guard(
    path: &Path,
    dir: &Path,
) -> Result<SessionRemovalGuard, RecoveryGcError> {
    let Ok(Some(meta)) = load_branch_meta(path) else {
        return Err(RecoveryGcError::NotCovered);
    };
    if !meta.recovered || meta.recovery_digest.trim().is_empty() {
        return Err(RecoveryGcError::NotCovered);
    }
    let Some(successor) = recovery_branch_successor(path, dir, &meta) else {
        return Err(RecoveryGcError::NotCovered);
    };
    let guard = try_acquire_session_removal_guard(&successor.to_string_lossy())
        .map_err(RecoveryGcError::Lease)?;
    if !recovery_branch_covered_by_successor(path, &successor, &meta) {
        guard.release();
        return Err(RecoveryGcError::NotCovered);
    }
    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::branch::{branch_id, save_branch_meta};
    use crate::sessionstore::eventlog::writer_id::new_session_writer_id;
    use crate::sessionstore::recovery_gc::recovery_branch_covered_by_parent;
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

    /// Go recovery_lineage_test.go `forkChain`: a root that
    /// diverges plus three chained recovery branches, each holding the
    /// previous content plus fresh turns.
    fn fork_chain(dir: &Path) -> (PathBuf, Vec<String>) {
        let root = dir.join("session.jsonl");
        let root_str = root.to_string_lossy().into_owned();
        let seed = Session::new("sys");
        seed.add(user("first"));
        seed.add(assistant("reply"));
        seed.save(&root_str).expect("save seed");
        let outside = load_session(&root_str).expect("load seed");
        outside.add(user("outside"));
        outside.save(&root_str).expect("diverge root");

        let mut branches: Vec<String> = Vec::new();
        let mut parent = root_str;
        let mut seed_msgs = seed.snapshot();
        for _ in 2..=4 {
            let live = Session::new("sys");
            for m in &seed_msgs {
                if m.role != Role::SYSTEM {
                    live.add(m.clone());
                }
            }
            live.add(user("turn"));
            live.add(assistant("reply"));
            live.set_recovery_lane(&new_session_writer_id());
            let info = live
                .save_recovery_branch(RecoveryBranchOptions {
                    original_path: parent.clone(),
                    ..RecoveryBranchOptions::default()
                })
                .expect("save recovery branch");
            assert_ne!(info.path, PathBuf::from(&parent));
            branches.push(info.path.to_string_lossy().into_owned());
            parent = info.path.to_string_lossy().into_owned();
            seed_msgs = live.snapshot();
        }
        (root, branches)
    }

    #[test]
    fn superseded_recovery_branch_is_reclaimable() {
        let dir = tempdir();
        let (_root, branches) = fork_chain(dir.path());
        assert_eq!(branches.len(), 3);
        for superseded in &branches[..branches.len() - 1] {
            let path = Path::new(superseded);
            assert!(
                !recovery_branch_covered_by_parent(path, dir.path()),
                "parent coverage should not hold; the root diverged"
            );
            assert!(
                recovery_branch_redundant(path, dir.path()),
                "an extended successor must cover {superseded}"
            );
        }
        let latest = Path::new(branches.last().expect("latest branch"));
        assert!(
            !recovery_branch_redundant(latest, dir.path()),
            "the latest branch holds turns no other session has"
        );
    }

    #[test]
    fn continued_recovery_branch_is_not_superseded() {
        let dir = tempdir();
        let (_root, branches) = fork_chain(dir.path());
        let live = load_session(&branches[0]).expect("load branch");
        live.add(user("kept working here"));
        live.save(&branches[0]).expect("save continued branch");
        assert!(
            !recovery_branch_redundant(Path::new(&branches[0]), dir.path()),
            "a continued branch is someone's conversation now"
        );
    }

    #[test]
    fn identical_recovery_copies_keep_one() {
        let dir = tempdir();
        let root = dir.path().join("session.jsonl");
        let root_str = root.to_string_lossy().into_owned();
        let seed = Session::new("sys");
        seed.add(user("root only"));
        seed.save(&root_str).expect("save root");

        let aaaa = dir.path().join("session-recovery-aaaa.jsonl");
        let bbbb = dir.path().join("session-recovery-bbbb.jsonl");
        for path in [&aaaa, &bbbb] {
            let live = Session::new("sys");
            live.add(user("unsaved"));
            live.add(assistant("work"));
            live.save(&path.to_string_lossy()).expect("save copy");
            let digest = digest_session_messages(&live.snapshot()).expect("digest");
            save_branch_meta(
                path,
                BranchMeta {
                    id: branch_id(&path.to_string_lossy()),
                    parent_id: branch_id(&root_str),
                    recovered: true,
                    recovery_digest: digest_string(&digest),
                    recovery_depth: 1,
                    ..BranchMeta::default()
                },
            )
            .expect("save meta");
        }
        assert!(
            recovery_branch_redundant(&aaaa, dir.path()),
            "the earlier copy is covered by its twin"
        );
        assert!(
            !recovery_branch_redundant(&bbbb, dir.path()),
            "the tie-break must keep exactly one of two identical copies"
        );
    }
}
