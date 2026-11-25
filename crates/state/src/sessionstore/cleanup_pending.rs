//! Delayed session cleanup (Go: the cleanup-pending family of
//! save.go plus empty_session_gc.go, orphan_outputs.go, and
//! fork_cleanup.go).
//!
//! Deleting a session outright races its writers, so the reference
//! defers deletion: a `.cleanup-pending.json` marker is published
//! beside the transcript first, and a later reconciliation pass —
//! holding the session's save and lease locks — finishes the
//! physical cleanup. The same pass reclaims sessions that never
//! gained user content, renames transcripts whose names grew too
//! long for their sidecars, drops stale lock sidecars, and sweeps
//! orphaned scratch-output spills of sessions that no longer exist.
//! Fork markers are handled inline under the removal guard; the
//! recovery-trash markers are delegated to their own reconciler.
//!
//! Contract: docs/contracts/session-format.md SS15.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, FixedOffset};

use reasonix_contract::provider::Role;

use super::branch::{branch_id, load_branch_meta, save_branch_meta_preserve_updated};
use super::recovery_gc::{go_dir, session_lease_held};
use super::recovery_trash::{
    join_error_strings, reconcile_recovery_trash_pending, reconcile_recovery_trash_stages,
};
use super::session::{
    canonical_session_save_path, is_cleanup_pending, load_session_messages, recovery_parent_stem,
    with_session_save_path_lock,
};
use super::session_lease::{
    remove_stale_session_lease_info_sidecar, remove_stale_session_lease_lock_sidecar,
    remove_stale_session_lock_sidecar, session_lease_held_by_other_runtime,
    session_lease_held_locally,
};
use super::session_lock::{SessionFileLockError, try_take_session_lock_file};
use super::session_order::{CleanupPendingMeta, session_content_mod_time, unix_millis_now};
use super::session_removal::try_acquire_session_removal_guard;
use crate::store::naming;

/// Operation recorded in a fork's cleanup-pending marker (Go:
/// `ForkPendingOperation`).
pub const FORK_PENDING_OPERATION: &str = "fork";

/// Grace period an empty session must stay untouched before the
/// reclaim scan may collect it (Go: `EmptySessionGracePeriod`).
pub const EMPTY_SESSION_GRACE_PERIOD: Duration = Duration::from_secs(5 * 60);

/// Grace period for orphaned outputs directories of deleted sessions
/// (Go: `orphanOutputsGrace`).
const ORPHAN_OUTPUTS_GRACE: Duration = Duration::from_secs(60 * 60);

/// Suffix of the delayed-cleanup marker (Go: `cleanupPendingExt`).
const CLEANUP_PENDING_EXT: &str = ".cleanup-pending.json";

/// Longest session basename that still leaves every sidecar name
/// inside the filesystem limit (Go: `maxSessionBasenameBytes`).
const MAX_SESSION_BASENAME_BYTES: usize = 224;

/// Suffix appended to a session path for its save-lock sidecar (Go:
/// `sessionLockSidecarSuffix`).
const SESSION_LOCK_SUFFIX: &str = ".lock";

/// Suffix appended for the lease-lock sidecar (Go:
/// `sessionLeaseLockSidecarSuffix`).
const SESSION_LEASE_LOCK_SUFFIX: &str = ".lease.lock";

/// Suffix appended for the lease-info sidecar (Go:
/// `sessionLeaseInfoSidecarSuffix`).
const SESSION_LEASE_INFO_SUFFIX: &str = ".lease.json";

/// The cleanup callback every reconciliation pass reports to (Go:
/// the `cleanup func(CleanupPendingInfo) error` parameter).
pub type CleanupCallback<'a> = Option<&'a mut dyn FnMut(CleanupPendingInfo) -> io::Result<()>>;

/// One recorded delayed-cleanup request (Go: `CleanupPendingInfo`).
#[derive(Debug, Clone)]
pub struct CleanupPendingInfo {
    /// Session the marker belongs to.
    pub session_path: String,
    /// The marker file itself.
    pub marker_path: String,
    /// Marker contents.
    pub meta: CleanupPendingMeta,
}

/// Remove the delayed-cleanup marker (Go: `ClearCleanupPending`),
/// called once the physical cleanup has finished.
pub fn clear_cleanup_pending(session_path: &str) -> io::Result<()> {
    let path = naming::session_cleanup_pending(Path::new(session_path));
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    super::v4_import::preserve_v4_removal(Path::new(session_path)).map_err(io::Error::other)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// List every cleanup-pending marker in `dir` (Go:
/// `ListCleanupPending`), sorted by session path. Unreadable or
/// blank markers decode to a zero-valued meta.
pub fn list_cleanup_pending(dir: &str) -> io::Result<Vec<CleanupPendingInfo>> {
    let dir = dir.trim();
    if dir.is_empty() {
        return Ok(Vec::new());
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(CLEANUP_PENDING_EXT) {
            continue;
        }
        let marker_path = Path::new(dir).join(name.as_ref());
        let raw = match reasonix_core::fileencoding::read_file_utf8(&marker_path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        let meta: CleanupPendingMeta = if raw.is_empty() {
            CleanupPendingMeta::default()
        } else {
            match serde_json::from_slice(&raw) {
                Ok(meta) => meta,
                Err(err) => {
                    return Err(io::Error::other(format!(
                        "read cleanup-pending marker {}: {err}",
                        marker_path.display()
                    )));
                }
            }
        };
        let session = format!("{}.jsonl", name.trim_end_matches(CLEANUP_PENDING_EXT));
        out.push(CleanupPendingInfo {
            session_path: Path::new(dir).join(session).to_string_lossy().into_owned(),
            marker_path: marker_path.to_string_lossy().into_owned(),
            meta,
        });
    }
    out.sort_by(|a, b| a.session_path.cmp(&b.session_path));
    Ok(out)
}

/// Reconcile every delayed cleanup in `dir` (Go:
/// `ReconcileCleanupPending`): session-directory upkeep first, then
/// stale-artifact sweeps, then each pending marker.
pub fn reconcile_cleanup_pending(dir: &str, mut cleanup: CleanupCallback<'_>) -> io::Result<()> {
    let mut errs: Vec<String> = Vec::new();
    if let Err(err) = reconcile_session_dir(dir, &mut cleanup) {
        errs.push(err.to_string());
    }
    if let Err(err) = reconcile_stale_artifacts(dir) {
        errs.push(err.to_string());
    }
    let pending = match list_cleanup_pending(dir) {
        Ok(pending) => pending,
        Err(err) => {
            errs.push(err.to_string());
            return join_error_strings(errs);
        }
    };
    for item in pending {
        let mut err: Option<io::Error> = None;
        match reconcile_pending(dir, &item, &mut cleanup) {
            Ok(true) => {}
            Ok(false) => match cleanup.as_mut() {
                Some(cb) => {
                    if let Err(e) = cb(item.clone()) {
                        err = Some(e);
                    }
                }
                None => continue,
            },
            Err(e) => err = Some(e),
        }
        if let Some(err) = err {
            errs.push(format!("{}: {err}", item.session_path));
        }
    }
    join_error_strings(errs)
}

/// Session-directory upkeep ahead of the pending sweep (Go:
/// `reconcileSessionDir`): stale sidecars first, then empty-session
/// reclaims. Skipped entirely when no cleanup callback is wired.
fn reconcile_session_dir(dir: &str, cleanup: &mut CleanupCallback<'_>) -> io::Result<()> {
    let mut errs: Vec<String> = Vec::new();
    if let Err(err) = reconcile_session_sidecars(dir) {
        errs.push(err.to_string());
    }
    if cleanup.is_none() {
        return join_error_strings(errs);
    }
    let now = chrono::Utc::now().fixed_offset();
    let empty = match reclaimable_empty_sessions(dir, &now, EMPTY_SESSION_GRACE_PERIOD) {
        Ok(empty) => empty,
        Err(err) => {
            errs.push(err.to_string());
            return join_error_strings(errs);
        }
    };
    let now = chrono::Utc::now().fixed_offset();
    for path in empty {
        if let Err(err) = reclaim_empty_session(&path, &now, cleanup) {
            errs.push(format!("{path}: {err}"));
        }
    }
    join_error_strings(errs)
}

/// Reclaim one empty session (Go: `reclaimEmptySession`): guard,
/// callback, then sidecar removal. A held lease is not an error —
/// another writer owns the session now.
fn reclaim_empty_session(
    path: &str,
    now: &DateTime<FixedOffset>,
    cleanup: &mut CleanupCallback<'_>,
) -> io::Result<()> {
    let guard = match try_acquire_session_removal_guard(path) {
        Ok(guard) => guard,
        Err(err) if err.is_held() => return Ok(()),
        Err(err) => return Err(io::Error::other(err.to_string())),
    };
    if !still_reclaimable_empty(Path::new(path), now, EMPTY_SESSION_GRACE_PERIOD) {
        guard.release();
        return Ok(());
    }
    let item = CleanupPendingInfo {
        session_path: path.to_string(),
        marker_path: String::new(),
        meta: CleanupPendingMeta {
            operation: "delete".to_string(),
            created_at: unix_millis_now(),
        },
    };
    if let Some(cb) = cleanup.as_mut()
        && let Err(err) = cb(item)
    {
        guard.release();
        return Err(err);
    }
    guard.remove_sidecars_and_release()
}

/// Whether `path` is still an untouched content-free session (Go:
/// `stillReclaimableEmpty`). Re-checked under the removal guard so a
/// session opened between listing and reclaim survives.
fn still_reclaimable_empty(path: &Path, now: &DateTime<FixedOffset>, grace: Duration) -> bool {
    let path_text = path.to_string_lossy();
    if is_cleanup_pending(&path_text) {
        return false;
    }
    let Some(mod_time) = session_content_mod_time(path) else {
        return false;
    };
    let Ok(grace) = chrono::Duration::from_std(grace) else {
        return false;
    };
    if *now - mod_time.get() < grace {
        return false;
    }
    !session_meta_counts_turns(path) && session_holds_no_user_message(path)
}

/// Empty sessions eligible for reclaim (Go:
/// `ReclaimableEmptySessions`), sorted by path.
pub fn reclaimable_empty_sessions(
    dir: &str,
    now: &DateTime<FixedOffset>,
    grace: Duration,
) -> io::Result<Vec<String>> {
    let dir = dir.trim();
    if dir.is_empty() {
        return Ok(Vec::new());
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if entry.file_type()?.is_dir() {
            continue;
        }
        if !naming::is_session_transcript_name(&name) || naming::is_subagent_transcript_name(&name)
        {
            continue;
        }
        let path = Path::new(dir).join(&name);
        let path_text = path.to_string_lossy().into_owned();
        if session_lease_held(&path_text) || !still_reclaimable_empty(&path, now, grace) {
            continue;
        }
        out.push(path_text);
    }
    out.sort();
    Ok(out)
}

/// Whether the session's sidecar meta counts turns (Go:
/// `sessionMetaCountsTurns`); an unreadable sidecar counts, so a
/// damaged session is never mistaken for an empty one.
fn session_meta_counts_turns(path: &Path) -> bool {
    match load_branch_meta(path) {
        Ok(Some(meta)) => meta.turns > 0,
        Ok(None) => false,
        Err(_) => true,
    }
}

/// Whether the session transcript holds no user message (Go:
/// `sessionHoldsNoUserMessage`). Damaged transcripts never count.
fn session_holds_no_user_message(path: &Path) -> bool {
    match load_session_messages(path) {
        Ok((messages, _, damaged)) => {
            if damaged {
                return false;
            }
            !messages.iter().any(|m| m.role == Role::USER)
        }
        Err(_) => false,
    }
}

/// Remove stale lock/lease sidecars of sessions in `dir` (Go:
/// `ReconcileSessionSidecars`), shortening overlong transcript names
/// first so their own sidecars exist to clean.
pub fn reconcile_session_sidecars(dir: &str) -> io::Result<()> {
    let dir = dir.trim();
    if dir.is_empty() {
        return Ok(());
    }
    let mut errs: Vec<String> = Vec::new();
    if let Err(err) = reconcile_overlong_session_filenames(dir) {
        errs.push(err.to_string());
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            errs.push(err.to_string());
            return join_error_strings(errs);
        }
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let sidecar = Path::new(dir).join(name.as_ref());
        // Dispatch on the full ".jsonl.*" suffixes so bare look-alike
        // files stay untouched; the trimmed stem keeps its ".jsonl".
        if let Some(stem) = name.strip_suffix(".jsonl.lease.json") {
            let base = Path::new(dir).join(format!("{stem}.jsonl"));
            if let Err(err) =
                remove_stale_session_lease_info_sidecar(&base.to_string_lossy(), &sidecar)
            {
                errs.push(format!("{}: {err}", sidecar.display()));
            }
        } else if let Some(stem) = name.strip_suffix(".jsonl.lease.lock") {
            let base = Path::new(dir).join(format!("{stem}.jsonl"));
            if let Err(err) =
                remove_stale_session_lease_lock_sidecar(&base.to_string_lossy(), &sidecar)
            {
                errs.push(format!("{}: {err}", sidecar.display()));
            }
        } else if let Some(stem) = name.strip_suffix(".jsonl.lock") {
            let base = Path::new(dir).join(format!("{stem}.jsonl"));
            if let Err(err) = remove_stale_session_lock_sidecar(&base.to_string_lossy(), &sidecar) {
                errs.push(format!("{}: {err}", sidecar.display()));
            }
        }
    }
    join_error_strings(errs)
}

/// Rename transcripts whose basenames outgrew their sidecars (Go:
/// `reconcileOverlongSessionFilenames`), then repoint branch
/// sidecars at the renamed parents.
fn reconcile_overlong_session_filenames(dir: &str) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let mut errs: Vec<String> = Vec::new();
    let mut renamed: Vec<(String, String)> = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if entry.file_type()?.is_dir() || !naming::is_session_transcript_name(&name) {
            continue;
        }
        let name = name.to_string_lossy();
        if name.len() <= MAX_SESSION_BASENAME_BYTES {
            continue;
        }
        let path = Path::new(dir).join(name.as_ref());
        let path_text = path.to_string_lossy().into_owned();
        if is_cleanup_pending(&path_text) {
            continue;
        }
        match rename_overlong_session(&path_text) {
            Ok(new_id) if !new_id.is_empty() => {
                renamed.push((branch_id(&path_text), new_id));
            }
            Ok(_) => {}
            Err(err) => errs.push(format!("{path_text}: {err}")),
        }
    }
    if !renamed.is_empty()
        && let Err(err) = reparent_session_branches(dir, &renamed)
    {
        errs.push(err.to_string());
    }
    join_error_strings(errs)
}

/// Whether the save-lock sidecar of `session_path` still fits the
/// filesystem name limit (Go: `sessionLockSidecarFits`).
fn session_lock_sidecar_fits(session_path: &str) -> bool {
    go_base_len(session_path) + SESSION_LOCK_SUFFIX.len()
        <= crate::sessionstore::eventlog::probe::NAME_MAX_BYTES
}

/// Whether the lease-lock sidecar of `session_path` still fits (Go:
/// `sessionLeaseSidecarFits`).
fn session_lease_sidecar_fits(session_path: &str) -> bool {
    go_base_len(session_path) + SESSION_LEASE_LOCK_SUFFIX.len()
        <= crate::sessionstore::eventlog::probe::NAME_MAX_BYTES
}

/// Byte length of `filepath.Base(path)`, with Go's "." fallback.
fn go_base_len(path: &str) -> usize {
    match Path::new(path).file_name() {
        Some(name) => name.len(),
        None if path == ".." => 2,
        None => 1,
    }
}

/// Rename one overlong transcript to its recovery-parent stem (Go:
/// `renameOverlongSession`), moving sidecars along. Returns the new
/// session id, or an empty string when the rename was skipped.
fn rename_overlong_session(old_path: &str) -> Result<String, io::Error> {
    let old_id = branch_id(old_path);
    let new_id = recovery_parent_stem(&old_id);
    if new_id == old_id {
        return Ok(String::new());
    }
    let new_path = Path::new(old_path)
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{new_id}.jsonl"));
    // Go filepath.Stat: an existing target aborts before any lock is
    // taken.
    match fs::metadata(&new_path) {
        Ok(_) => {
            return Err(io::Error::other(format!(
                "rename target {} already exists",
                new_path.display()
            )));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    let new_path_text = new_path.to_string_lossy().into_owned();
    // Both save-path locks held for the whole move, released LIFO.
    with_session_save_path_lock(old_path, || {
        with_session_save_path_lock(&new_path_text, || {
            rename_overlong_session_locked(old_path, &new_path_text, &new_id)
        })
    })
}

fn rename_overlong_session_locked(
    old_path: &str,
    new_path: &str,
    new_id: &str,
) -> Result<String, io::Error> {
    if session_lease_held_locally(old_path) {
        return Ok(String::new());
    }
    if session_lease_sidecar_fits(old_path) && session_lease_held_by_other_runtime(old_path) {
        return Ok(String::new());
    }
    let mut lock_file = None;
    if session_lock_sidecar_fits(old_path) {
        match try_take_session_lock_file(&naming::session_lock_file(Path::new(old_path))) {
            Ok(lock) => lock_file = Some(lock),
            Err(SessionFileLockError::Held) => return Ok(String::new()),
            Err(SessionFileLockError::Io(err)) => return Err(err),
        }
    }
    if let Err(err) = fs::rename(old_path, new_path) {
        if let Some(lock) = lock_file {
            lock.unlock();
        }
        return Err(err);
    }
    let mut errs: Vec<String> = Vec::new();
    if let Err(err) = migrate_session_sidecars(old_path, new_path, new_id) {
        errs.push(err.to_string());
    }
    if session_lease_sidecar_fits(old_path) {
        for sidecar in [
            format!("{old_path}{SESSION_LEASE_LOCK_SUFFIX}"),
            format!("{old_path}{SESSION_LEASE_INFO_SUFFIX}"),
        ] {
            if let Err(err) = fs::remove_file(&sidecar)
                && err.kind() != io::ErrorKind::NotFound
            {
                errs.push(err.to_string());
            }
        }
    }
    if let Some(lock) = lock_file
        && let Err(err) = lock.remove_and_unlock()
    {
        errs.push(err.to_string());
    }
    join_error_strings(errs).map(|()| new_id.to_string())
}

/// Move a session's sidecars to its new path (Go:
/// `migrateSessionSidecars`): meta first, then the paired moves.
fn migrate_session_sidecars(old_path: &str, new_path: &str, new_id: &str) -> io::Result<()> {
    let mut errs: Vec<String> = Vec::new();
    if go_base_len(old_path) + ".meta".len() <= crate::sessionstore::eventlog::probe::NAME_MAX_BYTES
    {
        match load_branch_meta(Path::new(old_path)) {
            Err(err) => errs.push(err.to_string()),
            Ok(Some(mut meta)) => {
                meta.id = new_id.to_string();
                match save_branch_meta_preserve_updated(Path::new(new_path), meta) {
                    Err(err) => errs.push(err.to_string()),
                    Ok(()) => {
                        if let Err(err) = fs::remove_file(naming::session_meta(Path::new(old_path)))
                            && err.kind() != io::ErrorKind::NotFound
                        {
                            errs.push(err.to_string());
                        }
                    }
                }
            }
            Ok(None) => {}
        }
    }
    for (src, dst) in naming::session_sidecar_moves(Path::new(old_path), Path::new(new_path)) {
        if go_base_len(&src.to_string_lossy())
            > crate::sessionstore::eventlog::probe::NAME_MAX_BYTES
        {
            continue;
        }
        if let Err(err) = fs::rename(&src, &dst)
            && err.kind() != io::ErrorKind::NotFound
        {
            errs.push(err.to_string());
        }
    }
    join_error_strings(errs)
}

/// Point branch metas at renamed parent sessions (Go:
/// `reparentSessionBranches`).
fn reparent_session_branches(dir: &str, renamed: &[(String, String)]) -> io::Result<()> {
    let mut errs: Vec<String> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir || !naming::is_session_transcript_name(&name) {
            continue;
        }
        let name = name.to_string_lossy();
        if name.len() + ".meta".len() > crate::sessionstore::eventlog::probe::NAME_MAX_BYTES {
            continue;
        }
        let path = Path::new(dir).join(name.as_ref());
        let path_text = path.to_string_lossy().into_owned();
        with_session_save_path_lock(&path_text, || match load_branch_meta(&path) {
            Ok(Some(mut meta)) => {
                if let Some((_, new_parent)) =
                    renamed.iter().find(|(old, _)| *old == meta.parent_id)
                    && *new_parent != meta.parent_id
                {
                    meta.parent_id = new_parent.clone();
                    if let Err(err) = save_branch_meta_preserve_updated(&path, meta) {
                        errs.push(format!("{path_text}: {err}"));
                    }
                }
            }
            Ok(None) => {}
            Err(err) => errs.push(format!("{path_text}: {err}")),
        });
    }
    join_error_strings(errs)
}

/// Delete outputs directories whose session no longer exists (Go:
/// `reconcileOrphanOutputs`).
fn reconcile_orphan_outputs(dir: &str) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let mut errs: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = Path::new(dir).join(entry.file_name());
        let transcript = naming::session_path_for_outputs_dir(&path);
        if transcript.as_os_str().is_empty() {
            continue;
        }
        // The transcript survived: not an orphan.
        match fs::metadata(&transcript) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            _ => continue,
        }
        let Ok(info) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = info.modified() else {
            continue;
        };
        let aged = SystemTime::now()
            .duration_since(modified)
            .map(|age| age >= ORPHAN_OUTPUTS_GRACE)
            .unwrap_or(false);
        if !aged {
            continue;
        }
        if let Err(err) = fs::remove_dir_all(&path)
            && err.kind() != io::ErrorKind::NotFound
        {
            errs.push(err.to_string());
        }
    }
    if let Err(err) = sweep_scratch_outputs() {
        errs.push(err.to_string());
    }
    join_error_strings(errs)
}

/// Delete scratch output spills older than the grace period (Go:
/// `sweepScratchOutputs`). Files and empty directories go; the
/// directory itself stays.
fn sweep_scratch_outputs() -> io::Result<()> {
    let dir = scratch_outputs_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let mut errs: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Ok(info) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = info.modified() else {
            continue;
        };
        let aged = SystemTime::now()
            .duration_since(modified)
            .map(|age| age >= ORPHAN_OUTPUTS_GRACE)
            .unwrap_or(false);
        if !aged {
            continue;
        }
        if let Err(err) = go_remove(&dir.join(entry.file_name()))
            && err.kind() != io::ErrorKind::NotFound
        {
            errs.push(err.to_string());
        }
    }
    join_error_strings(errs)
}

/// `os.Remove` semantics: files, symlinks, and empty directories.
fn go_remove(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path)? {
        info if info.is_dir() => fs::remove_dir(path),
        _ => fs::remove_file(path),
    }
}

/// Where spilled tool output lands before growing into a per-session
/// outputs directory (Go: `ScratchOutputsDir`).
pub fn scratch_outputs_dir() -> PathBuf {
    std::env::temp_dir().join("reasonix-outputs")
}

/// Stage and orphan cleanup ahead of the pending sweep (Go:
/// `reconcileStaleArtifacts`).
pub(crate) fn reconcile_stale_artifacts(dir: &str) -> io::Result<()> {
    let mut errs: Vec<String> = Vec::new();
    if let Err(err) = reconcile_recovery_trash_stages(Path::new(dir)) {
        errs.push(err.to_string());
    }
    if let Err(err) = reconcile_orphan_outputs(dir) {
        errs.push(err.to_string());
    }
    join_error_strings(errs)
}

/// Handle one pending marker (Go: `reconcilePending`). Returns
/// whether the item was handled here; `false` defers to the caller's
/// callback. Only fork markers are handled inline; recovery-trash
/// markers go to their own reconciler.
fn reconcile_pending(
    dir: &str,
    item: &CleanupPendingInfo,
    cleanup: &mut CleanupCallback<'_>,
) -> io::Result<bool> {
    if item.meta.operation != FORK_PENDING_OPERATION {
        return reconcile_recovery_trash_pending(item);
    }
    let Some(cb) = cleanup.as_mut() else {
        return Ok(true);
    };
    let dir = canonical_session_save_path(dir);
    let path = canonical_session_save_path(&item.session_path);
    let ext_ok = Path::new(&path)
        .extension()
        .is_some_and(|ext| ext == std::ffi::OsStr::new("jsonl"));
    if go_dir(Path::new(&path)) != Path::new(&dir) || !ext_ok {
        return Err(io::Error::other(
            "pending fork session is outside its session directory",
        ));
    }
    let want_marker = naming::session_cleanup_pending(Path::new(&path));
    if Path::new(&canonical_session_save_path(&item.marker_path)) != want_marker {
        return Err(io::Error::other(
            "pending fork marker does not belong to its session",
        ));
    }
    let mut item = item.clone();
    item.session_path = path.clone();
    let guard = match try_acquire_session_removal_guard(&path) {
        Ok(guard) => guard,
        Err(err) if err.is_held() => return Ok(true),
        Err(err) => return Err(io::Error::other(err.to_string())),
    };
    if guard.path() != path {
        guard.release();
        return Err(io::Error::other(
            "pending fork session changed during cleanup",
        ));
    }
    let marker = naming::session_cleanup_pending(Path::new(&path));
    let info = match fs::symlink_metadata(&marker) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            guard.release();
            return Ok(true);
        }
        Err(err) => {
            guard.release();
            return Err(err);
        }
        Ok(info) => info,
    };
    if !info.is_file() {
        guard.release();
        return Err(io::Error::other(
            "pending fork marker is not a regular file",
        ));
    }
    if let Err(err) = cb(item) {
        guard.release();
        return Err(err);
    }
    guard.remove_sidecars_and_release().map(|()| true)
}

#[cfg(test)]
#[path = "cleanup_pending_tests.rs"]
mod tests;
