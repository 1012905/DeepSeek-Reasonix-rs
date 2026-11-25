use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(test)]
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::util::{
    destination_exists, native_session, normalized_absolute, record_imported_title, same_directory,
    transform_and_copy_jsonl,
};
use crate::sessionstore::branch::{
    BranchMeta, BranchMetaError, branch_id, load_branch_meta, save_branch_meta,
};
use crate::sessionstore::eventlog::probe::NAME_MAX_BYTES;
use crate::sessionstore::session::{
    Session, digest_session_messages, load_session_unlocked, session_artifact_exists,
    with_session_save_path_lock,
};
use crate::sessionstore::session_lease::{
    session_lease_held_by_current_runtime, session_lease_held_by_other_runtime,
};
use crate::sessionstore::session_lock::lock_session_file;
use crate::store::{
    session_blobs_dir, session_event_log, session_lock_file, session_meta, session_sidecar_dirs,
    session_sidecar_files,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlatImportOutcome {
    NotApplicable,
    Fallback,
    Moved,
    Deferred,
    Blocked,
    Incomplete,
}

#[derive(Debug, Error)]
enum NativeMoveError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Branch(#[from] BranchMetaError),
}

#[cfg(test)]
pub(crate) type MoveHook = Box<dyn Fn(&Path) + Send>;
#[cfg(test)]
static MOVE_HOOK: OnceLock<Mutex<Option<MoveHook>>> = OnceLock::new();

#[cfg(test)]
pub(crate) fn set_move_hook(hook: Option<MoveHook>) {
    *MOVE_HOOK
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = hook;
}

#[cfg(test)]
fn run_move_hook(path: &Path) {
    let registry = MOVE_HOOK.get_or_init(Mutex::default);
    let hook = registry.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(hook) = hook.as_ref() {
        hook(path);
    }
}

pub(crate) fn rehome_flat_import(
    old: &Path,
    new: &Path,
    dest_dir: &Path,
    base: &str,
    summary: &str,
    legacy_info: Option<&fs::Metadata>,
    may_retire: bool,
) -> (usize, bool) {
    if !may_retire || same_directory(dest_dir, old.parent().unwrap_or_else(|| Path::new("."))) {
        return (0, false);
    }
    match move_flat_import(old, new, legacy_info) {
        FlatImportOutcome::Moved => {
            record_imported_title(dest_dir, base, summary);
            (1, false)
        }
        FlatImportOutcome::Deferred | FlatImportOutcome::Incomplete => (0, true),
        _ => (0, false),
    }
}

fn move_flat_import(
    old: &Path,
    new: &Path,
    legacy_info: Option<&fs::Metadata>,
) -> FlatImportOutcome {
    let Some(legacy_info) = legacy_info else {
        return FlatImportOutcome::NotApplicable;
    };
    let Ok(info) = fs::metadata(old) else {
        return FlatImportOutcome::NotApplicable;
    };
    if !mtime_matches(info.modified().ok(), legacy_info.modified().ok()) {
        return FlatImportOutcome::NotApplicable;
    }
    if destination_exists(new) && !native_session(&staged_session_path(new)) {
        return FlatImportOutcome::NotApplicable;
    }
    if lease_blocks_move(old) {
        return FlatImportOutcome::Deferred;
    }
    let outcome = move_native_flat_import(old, new);
    if matches!(
        outcome,
        FlatImportOutcome::Moved | FlatImportOutcome::Deferred | FlatImportOutcome::Incomplete
    ) {
        return outcome;
    }
    if matches!(outcome, FlatImportOutcome::Blocked) {
        return FlatImportOutcome::NotApplicable;
    }
    move_plain_flat_import(old, new)
}

fn mtime_matches(
    left: Option<std::time::SystemTime>,
    right: Option<std::time::SystemTime>,
) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    let difference = match left.duration_since(right) {
        Ok(difference) => difference,
        Err(error) => error.duration(),
    };
    difference <= Duration::from_secs(2)
}

fn lease_sidecar_fits(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.as_encoded_bytes().len() + ".lease.lock".len() <= NAME_MAX_BYTES)
}

fn lease_blocks_move(path: &Path) -> bool {
    session_lease_held_by_current_runtime(&path.to_string_lossy())
        || (lease_sidecar_fits(path)
            && session_lease_held_by_other_runtime(&path.to_string_lossy()))
}

fn move_native_flat_import(old: &Path, new: &Path) -> FlatImportOutcome {
    with_session_save_path_lock(&old.to_string_lossy(), || {
        if lease_blocks_move(old) {
            return FlatImportOutcome::Deferred;
        }
        if !native_session(old) {
            return FlatImportOutcome::Fallback;
        }
        let _old_file_lock = match lock_session_file(old) {
            Ok(guard) => guard,
            Err(_) => return FlatImportOutcome::Blocked,
        };
        with_session_save_path_lock(&new.to_string_lossy(), || {
            if let Some(parent) = new.parent()
                && fs::create_dir_all(parent).is_err()
            {
                return FlatImportOutcome::Blocked;
            }
            let _new_file_lock = match lock_session_file(new) {
                Ok(guard) => guard,
                Err(_) => return FlatImportOutcome::Blocked,
            };
            let stage = staged_session_path(new);
            let destination_present = session_artifact_exists(new);
            if destination_present && !native_session(&stage) {
                return FlatImportOutcome::Blocked;
            }
            let old_meta = match load_branch_meta(old) {
                Ok(meta) => meta,
                Err(_) => return prepublication_failure(destination_present, &stage),
            };
            let Ok(session) = load_session_unlocked(old) else {
                return prepublication_failure(destination_present, &stage);
            };
            if prepare_staged_session(&session, &stage).is_err() {
                return prepublication_failure(destination_present, &stage);
            }
            let result: Result<(), NativeMoveError> = publish_staged_session(&stage, new)
                .map_err(NativeMoveError::from)
                .and_then(|()| {
                    #[cfg(test)]
                    run_move_hook(old);
                    complete_native_branch_meta(old_meta, &stage, new)
                        .map_err(NativeMoveError::from)
                })
                .and_then(|()| retire_old_session(old).map_err(NativeMoveError::from));
            if result.is_ok() {
                cleanup_staged_session(&stage);
                FlatImportOutcome::Moved
            } else {
                FlatImportOutcome::Incomplete
            }
        })
    })
}

fn prepublication_failure(destination_present: bool, stage: &Path) -> FlatImportOutcome {
    if destination_present {
        FlatImportOutcome::Incomplete
    } else {
        cleanup_staged_session(stage);
        FlatImportOutcome::Blocked
    }
}

fn staged_session_path(new: &Path) -> PathBuf {
    let digest = Sha256::digest(normalized_absolute(new).to_string_lossy().as_bytes());
    let key: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    new.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".legacy-import.{key}.tmp"))
}

fn prepare_staged_session(session: &Session, stage: &Path) -> Result<(), FlatImportOutcome> {
    if native_session(stage) {
        let Ok(staged) = load_session_unlocked(stage) else {
            return Err(FlatImportOutcome::Blocked);
        };
        let same_content = matches!(
            (
                digest_session_messages(&session.snapshot()),
                digest_session_messages(&staged.snapshot()),
            ),
            (Ok(left), Ok(right)) if left == right
        );
        return if same_content {
            Ok(())
        } else {
            Err(FlatImportOutcome::Blocked)
        };
    }
    if session_artifact_exists(stage) {
        cleanup_staged_session(stage);
    }
    match session.save_if_absent(stage.to_string_lossy().as_ref()) {
        Ok(()) => Ok(()),
        Err(_) => Err(FlatImportOutcome::Blocked),
    }
}

fn complete_native_branch_meta(
    old_meta: Option<BranchMeta>,
    stage: &Path,
    new: &Path,
) -> Result<(), BranchMetaError> {
    if load_branch_meta(new)?.is_some() {
        return Ok(());
    }
    let mut meta = match old_meta {
        Some(meta) => meta,
        None => load_branch_meta(stage)?.unwrap_or_default(),
    };
    meta.id = branch_id(new.to_string_lossy().as_ref());
    save_branch_meta(new, meta)
}

fn publish_staged_session(stage: &Path, new: &Path) -> io::Result<()> {
    publish_directory_no_replace(&session_blobs_dir(stage), &session_blobs_dir(new))?;
    publish_file_no_replace(&session_event_log(stage), &session_event_log(new))?;
    publish_file_no_replace(stage, new)?;
    for (artifact, dest) in session_sidecar_files(stage)
        .into_iter()
        .zip(session_sidecar_files(new))
    {
        if artifact == session_meta(stage) || artifact == session_event_log(stage) {
            continue;
        }
        publish_file_no_replace(&artifact, &dest)?;
    }
    Ok(())
}

fn publish_directory_no_replace(source: &Path, dest: &Path) -> io::Result<()> {
    let metadata = match fs::metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "staged session blobs are not a directory",
        ));
    }
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source = entry.path();
        let dest = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            publish_directory_no_replace(&source, &dest)?;
        } else {
            publish_file_no_replace(&source, &dest)?;
        }
    }
    Ok(())
}

fn publish_file_no_replace(source: &Path, dest: &Path) -> io::Result<()> {
    let metadata = match fs::metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "staged session artifact is not a file",
        ));
    }
    match fs::hard_link(source, dest) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if same_file_contents(source, dest)? {
                Ok(())
            } else {
                Err(error)
            }
        }
        Err(error) => Err(error),
    }
}

fn same_file_contents(left: &Path, right: &Path) -> io::Result<bool> {
    let left_metadata = fs::metadata(left)?;
    let right_metadata = fs::metadata(right)?;
    if !left_metadata.is_file()
        || !right_metadata.is_file()
        || left_metadata.len() != right_metadata.len()
    {
        return Ok(false);
    }
    let mut left = File::open(left)?;
    let mut right = File::open(right)?;
    let mut left_buffer = [0_u8; 64 * 1024];
    let mut right_buffer = [0_u8; 64 * 1024];
    let mut offset = 0_u64;
    while offset < left_metadata.len() {
        let count = (left_metadata.len() - offset).min(64 * 1024) as usize;
        left.read_exact(&mut left_buffer[..count])?;
        right.read_exact(&mut right_buffer[..count])?;
        if left_buffer[..count] != right_buffer[..count] {
            return Ok(false);
        }
        offset += count as u64;
    }
    Ok(true)
}

fn retire_old_session(old: &Path) -> io::Result<()> {
    for artifact in session_sidecar_files(old) {
        match fs::remove_file(&artifact) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    match fs::remove_dir_all(session_blobs_dir(old)) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    match fs::remove_file(old) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

fn cleanup_staged_session(stage: &Path) {
    let artifacts = std::iter::once(stage.to_path_buf())
        .chain(session_sidecar_files(stage))
        .chain(std::iter::once(session_lock_file(stage)));
    for artifact in artifacts {
        let _ = fs::remove_file(artifact);
    }
    for directory in session_sidecar_dirs(stage) {
        let _ = fs::remove_dir_all(directory);
    }
}

fn move_plain_flat_import(old: &Path, new: &Path) -> FlatImportOutcome {
    let Some(parent) = new.parent() else {
        return FlatImportOutcome::NotApplicable;
    };
    if fs::create_dir_all(parent).is_err() {
        return FlatImportOutcome::NotApplicable;
    }
    match fs::hard_link(old, new) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return FlatImportOutcome::NotApplicable;
        }
        Err(_) => {
            if transform_and_copy_jsonl(old, new).is_err() {
                return FlatImportOutcome::NotApplicable;
            }
        }
    }
    if lease_blocks_move(old) {
        let _ = fs::remove_file(new);
        return FlatImportOutcome::Deferred;
    }
    if fs::remove_file(old).is_err() {
        return FlatImportOutcome::NotApplicable;
    }
    FlatImportOutcome::Moved
}
