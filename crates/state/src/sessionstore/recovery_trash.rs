//! Recovery-trash staging pipeline (Go: the trash family of
//! recovery_gc.go plus the sub-agent listing of subagent_records.go).
//!
//! A trash move is staged inside a hidden staging directory and
//! published with one atomic rename, so Desktop can never observe a
//! split transcript/sidecar set. The durable in-directory pending
//! marker is written before the first artifact moves, which lets a
//! later run finish an interrupted move; the pre-staging legacy
//! marker is reconciled for builds that wrote it.
//!
//! Contract: docs/contracts/session-format.md SS15.4.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use reasonix_contract::gojson::escape_html;
use reasonix_core::fileencoding::decode_to_utf8;
use reasonix_core::fileutil::{atomic_write_file_strict, mkdir_all_mode};

use super::branch::branch_id;
use super::cleanup_pending::{CleanupPendingInfo, clear_cleanup_pending};
use super::recovery_gc::{
    RECOVERY_TRASH_DIR, RECOVERY_TRASH_META_FILE, RECOVERY_TRASH_OPERATION_PREFIX,
    RECOVERY_TRASH_PENDING_FILE, RECOVERY_TRASH_STAGING_PREFIX,
};
use super::session_order::unix_millis_now;
use super::session_removal::{SessionRemovalGuard, try_acquire_session_removal_guard};
use crate::store::naming;

/// Go `recoveryTrashMeta`: per-item visibility metadata inside a
/// published trash entry.
#[derive(Debug, Serialize, Deserialize)]
pub struct RecoveryTrashMeta {
    #[serde(rename = "key")]
    pub key: String,
    #[serde(rename = "deletedAt")]
    pub deleted_at: i64,
}

/// Go `recoveryTrashPendingMeta`: the durable in-directory marker
/// of an interrupted staging move.
#[derive(Debug, Serialize, Deserialize)]
struct RecoveryTrashPendingMeta {
    #[serde(rename = "key")]
    key: String,
}

/// Go `SubagentArtifact` (listing shape): a persisted sub-agent
/// transcript and the metadata sidecar that owns it.
#[derive(Debug, Clone)]
pub struct SubagentArtifact {
    pub ref_: String,
    pub session_path: PathBuf,
    pub meta_path: PathBuf,
}

/// The `parentSession` field of Go `SubagentMeta`; the rest
/// of the record is irrelevant to ownership listing.
#[derive(Debug, Default, Deserialize)]
struct SubagentMetaLite {
    #[serde(rename = "parentSession", default)]
    parent_session: String,
}

/// Go `ValidSubagentRef`: the shape a sub-agent record reference
/// must have to be considered ours.
pub fn valid_subagent_ref(ref_: &str) -> bool {
    if !ref_.starts_with("sa_") {
        return false;
    }
    ref_.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Go `ListSubagentsByParent`: sub-agent artifacts whose metadata
/// declares the given parent session owner. An undecodable record is
/// skipped; a storage error aborts the listing.
pub fn list_subagents_by_parent(
    session_dir: &Path,
    parent_session: &str,
) -> io::Result<Vec<SubagentArtifact>> {
    let parent_session = parent_session.trim();
    if session_dir.to_string_lossy().trim().is_empty() || parent_session.is_empty() {
        return Ok(Vec::new());
    }
    let dir = session_dir.join("subagents");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir || !name.ends_with(".meta.json") {
            continue;
        }
        let ref_ = match name.strip_suffix(".meta.json") {
            Some(ref_) => ref_,
            None => continue,
        };
        if !valid_subagent_ref(ref_) {
            continue;
        }
        let meta_path = dir.join(&name);
        let bytes = std::fs::read(&meta_path)?;
        let text = String::from_utf8_lossy(&decode_to_utf8(&bytes)).into_owned();
        let Ok(meta) = serde_json::from_str::<SubagentMetaLite>(&text) else {
            continue;
        };
        if meta.parent_session.trim() != parent_session {
            continue;
        }
        out.push(SubagentArtifact {
            ref_: ref_.to_string(),
            session_path: dir.join(format!("{ref_}.jsonl")),
            meta_path,
        });
    }
    Ok(out)
}

/// Go `validRecoveryTrashKey`.
pub(crate) fn valid_recovery_trash_key(key: &str) -> bool {
    !key.is_empty()
        && Path::new(key)
            .file_name()
            .is_some_and(|n| n.to_str() == Some(key))
        && key != "."
        && key != ".."
        && key.ends_with(".jsonl")
        && !key.ends_with(".events.jsonl")
}

/// Go `moveRecoveryTrashPath`: move one artifact, tolerating a
/// blank or missing source; the destination parent is created on
/// demand.
fn move_recovery_trash_path(src: &Path, dst: &Path) -> io::Result<()> {
    if src.to_string_lossy().trim().is_empty() {
        return Ok(());
    }
    match std::fs::symlink_metadata(src) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    }
    let parent = match dst.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    mkdir_all_mode(parent, 0o755)?;
    std::fs::rename(src, dst)
}

/// Go `recoveryTrashSidecars`: every artifact a trash move
/// carries — the per-file sidecars, the inline telemetry sidecar, then
/// the sidecar directories.
fn recovery_trash_sidecars(path: &Path) -> Vec<PathBuf> {
    let mut artifacts = naming::session_sidecar_files(path);
    artifacts.push(PathBuf::from(format!(
        "{}.telemetry.json",
        path.to_string_lossy()
    )));
    artifacts.push(naming::session_checkpoint_dir(path));
    artifacts.push(naming::session_jobs_dir(path));
    artifacts.push(naming::session_inbox_dir(path));
    artifacts.push(naming::session_outputs_dir(path));
    artifacts.push(naming::session_blobs_dir(path));
    artifacts
}

fn move_recovery_trash_artifacts(dir: &Path, path: &Path, item_dir: &Path) -> io::Result<()> {
    for src in recovery_trash_sidecars(path) {
        let base = src.file_name().unwrap_or_default().to_os_string();
        move_recovery_trash_path(&src, &item_dir.join(base))?;
    }
    move_recovery_subagent_artifacts(dir, path, item_dir)
}

fn move_recovery_subagent_artifacts(dir: &Path, path: &Path, item_dir: &Path) -> io::Result<()> {
    let artifacts = list_subagents_by_parent(dir, &branch_id(&path.to_string_lossy()))?;
    let target_dir = item_dir.join("subagents");
    for artifact in artifacts {
        let mut paths = vec![artifact.session_path.clone(), artifact.meta_path.clone()];
        paths.extend(naming::session_sidecar_files(&artifact.session_path));
        paths.extend(naming::session_sidecar_dirs(&artifact.session_path));
        for src in paths {
            let base = src.file_name().unwrap_or_default().to_os_string();
            move_recovery_trash_path(&src, &target_dir.join(base))?;
        }
    }
    Ok(())
}

fn marshal_recovery_meta<T: Serialize>(value: &T) -> io::Result<String> {
    let json = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    Ok(escape_html(&json))
}

/// Go `writeRecoveryTrashPending`: the durable marker lands
/// before the first artifact moves.
pub(crate) fn write_recovery_trash_pending(item_dir: &Path, key: &str) -> io::Result<()> {
    if !valid_recovery_trash_key(key) {
        return Err(io::Error::other("invalid recovery trash key"));
    }
    let json = marshal_recovery_meta(&RecoveryTrashPendingMeta {
        key: key.to_string(),
    })?;
    atomic_write_file_strict(
        &item_dir.join(RECOVERY_TRASH_PENDING_FILE),
        json.as_bytes(),
        0o644,
    )
}

fn read_recovery_trash_pending(item_dir: &Path) -> io::Result<String> {
    let bytes = std::fs::read(item_dir.join(RECOVERY_TRASH_PENDING_FILE))?;
    let meta: RecoveryTrashPendingMeta =
        serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    Ok(meta.key)
}

fn clear_recovery_trash_pending(item_dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(item_dir.join(RECOVERY_TRASH_PENDING_FILE)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn recovery_trash_meta_json(key: &str) -> io::Result<String> {
    marshal_recovery_meta(&RecoveryTrashMeta {
        key: key.to_string(),
        deleted_at: unix_millis_now(),
    })
}

/// Go `writeRecoveryTrashMeta`: creates the item directory.
fn write_recovery_trash_meta(item_dir: &Path, key: &str) -> io::Result<()> {
    let json = recovery_trash_meta_json(key)?;
    mkdir_all_mode(item_dir, 0o755)?;
    std::fs::write(item_dir.join(RECOVERY_TRASH_META_FILE), json.as_bytes())
}

/// Go `writeRecoveryTrashMetaExisting`: a missing item directory
/// propagates so callers can detect the restore/purge race.
fn write_recovery_trash_meta_existing(item_dir: &Path, key: &str) -> io::Result<()> {
    let json = recovery_trash_meta_json(key)?;
    std::fs::write(item_dir.join(RECOVERY_TRASH_META_FILE), json.as_bytes())
}

/// Go `reserveRecoveryTrashStage`: a fresh staging directory
/// inside the trash root.
pub(crate) fn reserve_recovery_trash_stage(dir: &Path) -> io::Result<PathBuf> {
    let root = dir.join(RECOVERY_TRASH_DIR);
    mkdir_all_mode(&root, 0o755)?;
    let stamp = unix_millis_now();
    for i in 0..10000u32 {
        let candidate = root.join(format!("{RECOVERY_TRASH_STAGING_PREFIX}{stamp}-{i}"));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("could not reserve recovery trash stage"))
}

fn key_stem(key: &str) -> String {
    match Path::new(key).extension() {
        Some(ext) => key
            .strip_suffix(&format!(".{}", ext.to_string_lossy()))
            .unwrap_or(key)
            .to_string(),
        None => key.to_string(),
    }
}

/// Go `publishRecoveryTrashStage`: one atomic rename into the
/// visible trash layout, suffixed past collisions.
fn publish_recovery_trash_stage(dir: &Path, key: &str, stage_dir: &Path) -> io::Result<PathBuf> {
    let root = dir.join(RECOVERY_TRASH_DIR);
    let stem = key_stem(key);
    let stamp = unix_millis_now();
    for i in 0..1000u32 {
        let name = if i == 0 {
            key.to_string()
        } else {
            format!("{stem}-recovery-{stamp}-{i}")
        };
        let item_dir = root.join(&name);
        match std::fs::symlink_metadata(&item_dir) {
            Ok(_) => continue,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        match std::fs::rename(stage_dir, &item_dir) {
            Ok(()) => return Ok(item_dir),
            Err(e) => match std::fs::symlink_metadata(&item_dir) {
                Ok(_) => continue, // another producer won this candidate
                Err(se) if se.kind() == io::ErrorKind::NotFound => return Err(e),
                Err(se) => return Err(se),
            },
        }
    }
    Err(io::Error::other("could not publish recovery trash target"))
}

/// Go `prepareRecoveryTrashStage`: marker first, then the
/// transcript move — a crash is recoverable from disk alone.
pub(crate) fn prepare_recovery_trash_stage(
    path: &Path,
    key: &str,
    stage_dir: &Path,
) -> io::Result<()> {
    write_recovery_trash_pending(stage_dir, key)?;
    move_recovery_trash_path(path, &stage_dir.join(key))
}

/// Go `prepareRecoveryTrashEntry`: the pre-staging legacy
/// protocol.
// No production caller: the staging pipeline superseded this entry
// path, but the legacy surface stays complete for the reconciliation
// tests (Go keeps the same helpers).
#[allow(dead_code)]
pub(crate) fn prepare_recovery_trash_entry(
    path: &Path,
    key: &str,
    item_dir: &Path,
) -> io::Result<()> {
    write_recovery_trash_meta(item_dir, key)?;
    move_recovery_trash_path(path, &item_dir.join(key))
}

/// Go `reserveRecoveryTrashItemDir`.
// Test-only alongside prepare_recovery_trash_entry above.
#[allow(dead_code)]
pub(crate) fn reserve_recovery_trash_item_dir(
    dir: &Path,
    key: &str,
) -> io::Result<(String, PathBuf)> {
    let root = dir.join(RECOVERY_TRASH_DIR);
    mkdir_all_mode(&root, 0o755)?;
    let stem = key_stem(key);
    for i in 0..1000u32 {
        let name = if i == 0 {
            key.to_string()
        } else {
            format!("{stem}-recovery-{}-{i}", unix_millis_now())
        };
        let item_dir = root.join(&name);
        match std::fs::create_dir(&item_dir) {
            Ok(()) => return Ok((name, item_dir)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("could not reserve recovery trash target"))
}

/// Go `finishRecoveryTrashStage`: publish the staged move, then
/// finish the entry. Consumes the branch guard on every path.
pub(crate) fn finish_recovery_trash_stage(
    dir: &Path,
    path: &Path,
    key: &str,
    stage_dir: &Path,
    guard: SessionRemovalGuard,
) -> io::Result<()> {
    if let Err(e) = move_recovery_trash_artifacts(dir, path, stage_dir) {
        guard.release();
        return Err(e);
    }
    let item_dir = match publish_recovery_trash_stage(dir, key, stage_dir) {
        Ok(item_dir) => item_dir,
        Err(e) => {
            guard.release();
            return Err(e);
        }
    };
    if let Err(e) = write_recovery_trash_meta_existing(&item_dir, key) {
        if e.kind() != io::ErrorKind::NotFound {
            guard.release();
            return Err(e);
        }
        // A complete entry may be restored or purged as soon as it is
        // published; nothing is left for this producer to finalize.
        return guard.remove_sidecars_and_release();
    }
    if let Err(e) = clear_recovery_trash_pending(&item_dir) {
        guard.release();
        return Err(e);
    }
    guard.remove_sidecars_and_release()
}

/// Go `finishRecoveryTrashMove`: the legacy direct move, holding
/// the branch guard until the entry is complete and the hidden marker
/// is cleared.
fn finish_recovery_trash_move(
    dir: &Path,
    path: &Path,
    key: &str,
    item_dir: &Path,
    guard: SessionRemovalGuard,
) -> io::Result<()> {
    if let Err(e) = mkdir_all_mode(item_dir, 0o755) {
        guard.release();
        return Err(e);
    }
    if let Err(e) = move_recovery_trash_artifacts(dir, path, item_dir) {
        guard.release();
        return Err(e);
    }
    if let Err(e) = write_recovery_trash_meta(item_dir, key) {
        guard.release();
        return Err(e);
    }
    if let Err(e) = clear_cleanup_pending(&path.to_string_lossy()) {
        guard.release();
        return Err(e);
    }
    guard.remove_sidecars_and_release()
}

/// Go `reconcileRecoveryTrashPending`: completes an interrupted
/// move left by the pre-staging protocol. Returns false when the
/// marker is not a trash marker.
pub(crate) fn reconcile_recovery_trash_pending(
    item: &CleanupPendingInfo,
) -> Result<bool, io::Error> {
    let operation = item.meta.operation.trim();
    let Some(item_name) = operation.strip_prefix(RECOVERY_TRASH_OPERATION_PREFIX) else {
        return Ok(false);
    };
    if item_name.is_empty()
        || !Path::new(item_name)
            .file_name()
            .is_some_and(|n| n.to_str() == Some(item_name))
        || item_name == "."
        || item_name == ".."
    {
        return Err(io::Error::other("invalid recovery trash target"));
    }
    let path = PathBuf::from(&item.session_path);
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let key = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let guard = try_acquire_session_removal_guard(&path.to_string_lossy())
        .map_err(|e| io::Error::other(e.to_string()))?;
    finish_recovery_trash_move(
        &dir,
        &path,
        &key,
        &dir.join(RECOVERY_TRASH_DIR).join(item_name),
        guard,
    )
    .map(|()| true)
}

/// Go `reconcileRecoveryTrashStages`: finishes staged moves
/// interrupted between the durable marker and the atomic publish.
pub(crate) fn reconcile_recovery_trash_stages(dir: &Path) -> io::Result<()> {
    if dir.to_string_lossy().trim().is_empty() {
        return Ok(());
    }
    let root = dir.join(RECOVERY_TRASH_DIR);
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut errs: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let item_dir = root.join(entry.file_name());
        match std::fs::metadata(item_dir.join(RECOVERY_TRASH_PENDING_FILE)) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                errs.push(format!(
                    "inspect recovery trash stage {}: {e}",
                    item_dir.to_string_lossy()
                ));
                continue;
            }
        }
        if let Err(e) = reconcile_recovery_trash_stage(dir, &item_dir) {
            errs.push(format!(
                "reconcile recovery trash stage {}: {e}",
                item_dir.to_string_lossy()
            ));
        }
    }
    join_error_strings(errs)
}

fn reconcile_recovery_trash_stage(dir: &Path, item_dir: &Path) -> io::Result<()> {
    let key = read_recovery_trash_pending(item_dir)?.trim().to_string();
    if !valid_recovery_trash_key(&key) {
        return Err(io::Error::other("invalid recovery trash key"));
    }
    let staged = regular_recovery_trash_path(&item_dir.join(&key))?;
    let item_name = item_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // A non-staging name means the atomic rename already succeeded;
    // only visibility metadata or final marker cleanup may remain.
    if !item_name.starts_with(RECOVERY_TRASH_STAGING_PREFIX) {
        if !staged {
            return Err(io::Error::other(
                "published recovery trash item is missing transcript",
            ));
        }
        match std::fs::metadata(item_dir.join(RECOVERY_TRASH_META_FILE)) {
            Ok(_) => return clear_recovery_trash_pending(item_dir),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        match write_recovery_trash_meta_existing(item_dir, &key) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(()); // restored or purged after complete publication
            }
            Err(e) => return Err(e),
        }
        return clear_recovery_trash_pending(item_dir);
    }
    let live_path = dir.join(&key);
    let live = regular_recovery_trash_path(&live_path)?;
    match (live, staged) {
        // The durable marker landed but the first rename did not: no
        // artifact moved, so discard the empty stage and let a later
        // pass revalidate coverage before trying again.
        (true, false) => remove_empty_recovery_trash_stage(item_dir),
        (true, true) => Err(io::Error::other(
            "live and staged recovery transcripts both exist",
        )),
        (false, false) => Err(io::Error::other(
            "recovery trash stage is missing transcript",
        )),
        (false, true) => {
            let guard = try_acquire_session_removal_guard(&live_path.to_string_lossy())
                .map_err(|e| io::Error::other(e.to_string()))?;
            finish_recovery_trash_stage(dir, &live_path, &key, item_dir, guard)
        }
    }
}

fn regular_recovery_trash_path(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(md) if md.is_file() => Ok(true),
        Ok(_) => Err(io::Error::other(format!(
            "recovery trash path is not a regular file: {}",
            path.to_string_lossy()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn remove_empty_recovery_trash_stage(item_dir: &Path) -> io::Result<()> {
    let mut entries: Vec<(String, bool)> = Vec::new();
    for entry in std::fs::read_dir(item_dir)? {
        let entry = entry?;
        entries.push((
            entry.file_name().to_string_lossy().into_owned(),
            entry.file_type().map(|t| t.is_dir()).unwrap_or(false),
        ));
    }
    if entries.len() != 1 || entries[0].0 != RECOVERY_TRASH_PENDING_FILE || entries[0].1 {
        return Err(io::Error::other(
            "recovery trash stage contains artifacts without a transcript",
        ));
    }
    std::fs::remove_dir_all(item_dir)
}

/// Joins collected per-item error texts the way Go `errors.Join`
/// renders them (newline-separated); an empty list is success.
pub(crate) fn join_error_strings(errs: Vec<String>) -> io::Result<()> {
    if errs.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(errs.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> tempfile::TempDir {
        crate::sessionstore::eventlog::test_support::tempdir()
    }

    #[test]
    fn valid_recovery_trash_key_table() {
        assert!(valid_recovery_trash_key("a.jsonl"));
        assert!(valid_recovery_trash_key("session-recovery-1.jsonl"));
        assert!(!valid_recovery_trash_key(""));
        assert!(!valid_recovery_trash_key("."));
        assert!(!valid_recovery_trash_key(".."));
        assert!(!valid_recovery_trash_key("a.events.jsonl"));
        assert!(!valid_recovery_trash_key("sub/a.jsonl"));
        assert!(!valid_recovery_trash_key("a.txt"));
    }

    #[test]
    fn move_recovery_trash_path_tolerates_missing_and_moves() {
        let dir = tempdir();
        let src = dir.path().join("src.txt");
        // A missing source is a no-op.
        move_recovery_trash_path(&src, &dir.path().join("dst.txt")).expect("missing src ok");
        std::fs::write(&src, b"x").expect("write src");
        move_recovery_trash_path(&src, &dir.path().join("nested").join("dst.txt"))
            .expect("move with fresh parent");
        assert!(dir.path().join("nested").join("dst.txt").exists());
        assert!(!src.exists());
    }

    #[test]
    fn pending_marker_round_trip() {
        let dir = tempdir();
        write_recovery_trash_pending(dir.path(), "key.jsonl").expect("write pending");
        assert_eq!(
            read_recovery_trash_pending(dir.path()).expect("read pending"),
            "key.jsonl"
        );
        assert!(dir.path().join(RECOVERY_TRASH_PENDING_FILE).exists());
        clear_recovery_trash_pending(dir.path()).expect("clear pending");
        assert!(!dir.path().join(RECOVERY_TRASH_PENDING_FILE).exists());
        // Clearing again is a no-op.
        clear_recovery_trash_pending(dir.path()).expect("clear again");
        // Invalid keys are refused.
        assert!(write_recovery_trash_pending(dir.path(), "bad").is_err());
    }

    #[test]
    fn publish_recovery_trash_stage_collides_with_suffix() {
        let dir = tempdir();
        // The first candidate name is already occupied.
        std::fs::create_dir_all(dir.path().join(RECOVERY_TRASH_DIR).join("key.jsonl"))
            .expect("occupy first candidate");
        let stage = reserve_recovery_trash_stage(dir.path()).expect("reserve stage");
        let published = publish_recovery_trash_stage(dir.path(), "key.jsonl", &stage)
            .expect("publish past collision");
        let name = published
            .file_name()
            .expect("published name")
            .to_string_lossy()
            .into_owned();
        assert_ne!(name, "key.jsonl");
        assert!(name.starts_with("key-recovery-"));
        assert!(published.exists());
    }

    #[test]
    fn trash_meta_shape_has_deleted_at() {
        let dir = tempdir();
        write_recovery_trash_meta(dir.path(), "key.jsonl").expect("write meta");
        let text =
            std::fs::read_to_string(dir.path().join(RECOVERY_TRASH_META_FILE)).expect("read meta");
        assert!(text.contains("\"deletedAt\""));
        assert!(text.contains("\"key\""));
        // The existing-only variant refuses to create the directory.
        assert!(
            write_recovery_trash_meta_existing(&dir.path().join("missing"), "k.jsonl").is_err()
        );
    }

    #[test]
    fn list_subagents_by_parent_filters_by_owner() {
        let dir = tempdir();
        let sub = dir.path().join("subagents");
        std::fs::create_dir_all(&sub).expect("mkdir subagents");
        std::fs::write(
            sub.join("sa_one.meta.json"),
            br#"{"parentSession":"  owner  "}"#,
        )
        .expect("write one");
        std::fs::write(
            sub.join("sa_two.meta.json"),
            br#"{"parentSession":"other"}"#,
        )
        .expect("write two");
        std::fs::write(sub.join("sa_bad.meta.json"), b"not json").expect("write bad");
        std::fs::write(
            sub.join("invalid.meta.json"),
            br#"{"parentSession":"owner"}"#,
        )
        .expect("write invalid ref");
        let out = list_subagents_by_parent(dir.path(), "owner").expect("list");
        assert_eq!(out.len(), 1, "undecodable and foreign records are skipped");
        assert_eq!(out[0].ref_, "sa_one");
        assert_eq!(out[0].session_path, sub.join("sa_one.jsonl"));
        assert_eq!(out[0].meta_path, sub.join("sa_one.meta.json"));
        assert!(
            list_subagents_by_parent(&dir.path().join("missing"), "owner")
                .expect("missing dir")
                .is_empty()
        );
    }
}
