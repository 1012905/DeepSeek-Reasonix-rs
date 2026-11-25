use std::path::{Path, PathBuf};

use crate::sessionstore::eventlog::record::CreatedAt;

use super::path_capture::{Fingerprint, fingerprint_path};
use super::secure_path;
use super::store::Store;
use super::transaction::{
    ConversationApplier, ConversationError, ConversationPhase, TransactionError,
    TransactionInjection, TransactionPhase,
};
use super::transaction_prepare::{new_transaction_id, persist_manifest, sibling_paths};
use super::types::{
    FileStage, RewindPlan, RewindScope, TransactionManifest, TransactionState, TransactionTarget,
};

pub(super) fn compensate(
    store: &Store,
    targets: &[TransactionTarget],
    stages: &mut Vec<FileStage>,
    injection: Option<&TransactionInjection>,
) -> Result<(), TransactionError> {
    let mut errors = Vec::new();
    for target in targets.iter().rev() {
        if injection.is_some_and(|value| value.fails(TransactionPhase::CompensateFile)) {
            let error = TransactionError::injected(TransactionPhase::CompensateFile);
            mark_stage(stages, target, Some(&error));
            errors.push(error);
            continue;
        }
        let result = if target.published {
            compensate_published(store, target)
        } else {
            cleanup_temp(store, target)
        };
        if let Err(error) = result {
            mark_stage(stages, target, Some(&error));
            errors.push(error);
        } else {
            mark_stage(stages, target, None);
        }
    }
    if !errors.is_empty() {
        return Err(TransactionError::Compensation(errors));
    }
    store
        .sync_forward_ownership(targets)
        .map_err(|error| TransactionError::at(TransactionPhase::SyncForwardOwnership, error.into()))
}

fn compensate_published(store: &Store, target: &TransactionTarget) -> Result<(), TransactionError> {
    let current = fingerprint_path(&store.root, Path::new(&target.abs_path))
        .map_err(|error| TransactionError::at(TransactionPhase::CompensateFile, error.into()))?;
    if fingerprint_matches(
        &current,
        target.forward_existed,
        &target.forward_sha,
        target.forward_mode,
    ) {
        return cleanup_temp(store, target);
    }

    if target.forward_existed
        && !current.existed
        && !target.backup_path.is_empty()
        && crash_window_matches(store, target)?
    {
        secure_path::rename(
            &store.root,
            Path::new(&target.backup_path),
            Path::new(&target.abs_path),
        )
        .map_err(|error| TransactionError::at(TransactionPhase::CompensateFile, error.into()))?;
        return cleanup_temp(store, target);
    }

    if target.forward_existed {
        let forward = match load_forward(store, target) {
            Ok(data) => data,
            Err(error) if !target.backup_path.is_empty() => {
                match secure_path::read_file(&store.root, Path::new(&target.backup_path)) {
                    Ok(data) => data,
                    Err(backup_error) => {
                        return Err(TransactionError::Compensation(vec![
                            error,
                            TransactionError::at(
                                TransactionPhase::CompensateFile,
                                backup_error.into(),
                            ),
                        ]));
                    }
                }
            }
            Err(error) => {
                return Err(TransactionError::at(
                    TransactionPhase::CompensateFile,
                    error,
                ));
            }
        };
        if !fingerprint_matches(
            &current,
            target.restore_existed,
            &target.restore_sha,
            target.restore_mode,
        ) {
            let recovery = recovery_path(target);
            secure_path::write_new_file(
                &store.root,
                &recovery,
                &forward,
                effective_mode(target.forward_mode),
            )
            .map_err(|error| {
                TransactionError::at(TransactionPhase::CompensateFile, error.into())
            })?;
            return Err(TransactionError::ExternalModification {
                path: target.path.clone(),
                recovery: Some(recovery),
            });
        }
        let backup_exists = secure_path::path_exists(&store.root, Path::new(&target.backup_path))
            .map_err(|error| {
            TransactionError::at(TransactionPhase::CompensateFile, error.into())
        })?;
        if backup_exists {
            if current.existed {
                secure_path::remove(&store.root, Path::new(&target.abs_path)).map_err(|error| {
                    TransactionError::at(TransactionPhase::CompensateFile, error.into())
                })?;
            }
            return secure_path::rename(
                &store.root,
                Path::new(&target.backup_path),
                Path::new(&target.abs_path),
            )
            .map_err(|error| TransactionError::at(TransactionPhase::CompensateFile, error.into()));
        }
        return restore_forward_copy(store, target, &forward);
    }

    if !fingerprint_matches(
        &current,
        target.restore_existed,
        &target.restore_sha,
        target.restore_mode,
    ) {
        return Err(TransactionError::ExternalModification {
            path: target.path.clone(),
            recovery: None,
        });
    }
    match secure_path::remove(&store.root, Path::new(&target.abs_path)) {
        Ok(()) => Ok(()),
        Err(error) if error.is_not_found() => Ok(()),
        Err(error) => Err(TransactionError::at(
            TransactionPhase::CompensateFile,
            error.into(),
        )),
    }
}

fn crash_window_matches(
    store: &Store,
    target: &TransactionTarget,
) -> Result<bool, TransactionError> {
    let backup = fingerprint_path(&store.root, Path::new(&target.backup_path))
        .map_err(|error| TransactionError::at(TransactionPhase::CompensateFile, error.into()))?;
    if !fingerprint_matches(&backup, true, &target.forward_sha, target.forward_mode) {
        return Ok(false);
    }
    if target.action == "delete" {
        return Ok(true);
    }
    if target.publish_tmp.is_empty() {
        return Ok(false);
    }
    secure_path::path_exists(&store.root, Path::new(&target.publish_tmp))
        .map_err(|error| TransactionError::at(TransactionPhase::CompensateFile, error.into()))
}

fn restore_forward_copy(
    store: &Store,
    target: &TransactionTarget,
    forward: &[u8],
) -> Result<(), TransactionError> {
    if secure_path::path_exists(&store.root, Path::new(&target.abs_path))
        .map_err(|error| TransactionError::at(TransactionPhase::CompensateFile, error.into()))?
    {
        secure_path::remove(&store.root, Path::new(&target.abs_path)).map_err(|error| {
            TransactionError::at(TransactionPhase::CompensateFile, error.into())
        })?;
    }
    let now = CreatedAt::now();
    let plan = RewindPlan {
        plan_id: format!(
            "compensate-{}",
            now.get().timestamp_nanos_opt().unwrap_or_default()
        ),
        ..RewindPlan::default()
    };
    let id = new_transaction_id(&plan, now);
    let (temp, _) = sibling_paths(Path::new(&target.abs_path), &id, 0)?;
    let result = (|| {
        secure_path::write_new_file(
            &store.root,
            &temp,
            forward,
            effective_mode(target.forward_mode),
        )?;
        secure_path::rename(&store.root, &temp, Path::new(&target.abs_path))
    })();
    if result.is_err() {
        let _ = secure_path::remove(&store.root, &temp);
    }
    result.map_err(|error| TransactionError::at(TransactionPhase::CompensateFile, error.into()))
}

fn load_forward(store: &Store, target: &TransactionTarget) -> Result<Vec<u8>, TransactionError> {
    if target.forward_blob.is_empty() {
        return Ok(target.forward_inline.clone());
    }
    let blobs = store.blobs.as_ref().ok_or_else(|| {
        TransactionError::at(
            TransactionPhase::CompensateFile,
            TransactionError::MissingPayload {
                path: target.path.clone(),
                reference: target.forward_blob.clone(),
            },
        )
    })?;
    blobs
        .get(&target.forward_blob)
        .map_err(|error| TransactionError::at(TransactionPhase::CompensateFile, error.into()))
}

fn cleanup_temp(store: &Store, target: &TransactionTarget) -> Result<(), TransactionError> {
    if target.publish_tmp.is_empty() {
        return Ok(());
    }
    match secure_path::remove(&store.root, Path::new(&target.publish_tmp)) {
        Ok(()) => Ok(()),
        Err(error) if error.is_not_found() => Ok(()),
        Err(error) => Err(TransactionError::at(
            TransactionPhase::CompensateFile,
            error.into(),
        )),
    }
}

fn recovery_path(target: &TransactionTarget) -> PathBuf {
    let suffix = target.restore_sha.get(..8).unwrap_or("unknown");
    PathBuf::from(format!("{}.reasonix-recovery-{suffix}", target.abs_path))
}

fn effective_mode(mode: u32) -> u32 {
    if mode == 0 { 0o644 } else { mode }
}

fn fingerprint_matches(fingerprint: &Fingerprint, existed: bool, sha: &str, mode: u32) -> bool {
    if fingerprint.existed != existed {
        return false;
    }
    if !existed {
        return true;
    }
    if !sha.is_empty() && fingerprint.sha256 != sha {
        return false;
    }
    mode == 0 || fingerprint.mode == 0 || fingerprint.mode == mode
}

fn mark_stage(
    stages: &mut Vec<FileStage>,
    target: &TransactionTarget,
    error: Option<&TransactionError>,
) {
    if !stages.iter().any(|stage| stage.path == target.path) {
        stages.push(FileStage {
            path: target.path.clone(),
            phase: "compensate".to_owned(),
            action: target.action.clone(),
            ..FileStage::default()
        });
    }
    let stage = stages
        .iter_mut()
        .find(|stage| stage.path == target.path)
        .expect("compensation stage");
    stage.phase = "compensate".to_owned();
    stage.compensated = error.is_none();
    stage.comp_error = error.map(ToString::to_string).unwrap_or_default();
}

pub(super) fn restore_conversation(
    transaction: &TransactionManifest,
    applier: Option<&dyn ConversationApplier>,
) -> Result<(), TransactionError> {
    if !matches!(
        transaction.scope,
        RewindScope::Conversation | RewindScope::Both
    ) {
        return Ok(());
    }
    let Some(applier) = applier else {
        return Ok(());
    };
    let mut errors = Vec::new();
    if !transaction.conversation_forward.is_empty()
        && let Err(error) = applier.restore_conversation(&transaction.conversation_forward)
    {
        errors.push(conversation_error(
            ConversationPhase::RestoreConversation,
            error,
        ));
    }
    if !transaction.checkpoint_backup.is_empty()
        && let Err(error) = applier.restore_checkpoints(&transaction.checkpoint_backup)
    {
        errors.push(conversation_error(
            ConversationPhase::RestoreCheckpoints,
            error,
        ));
    }
    combine_errors(errors)
}

fn conversation_error(phase: ConversationPhase, source: ConversationError) -> TransactionError {
    TransactionError::Conversation { phase, source }
}

fn combine_errors(errors: Vec<TransactionError>) -> Result<(), TransactionError> {
    match errors.len() {
        0 => Ok(()),
        1 => Err(errors.into_iter().next().expect("single error")),
        _ => Err(TransactionError::Compensation(errors)),
    }
}

pub(super) fn fail(
    store: &Store,
    transaction: &mut TransactionManifest,
    mut stages: Vec<FileStage>,
    cause: TransactionError,
    applier: Option<&dyn ConversationApplier>,
    restore_state: bool,
    injection: Option<&TransactionInjection>,
) -> Result<(), TransactionError> {
    let mut compensation_errors = Vec::new();
    if restore_state && let Err(error) = restore_conversation(transaction, applier) {
        compensation_errors.push(error);
    }
    if let Err(error) = compensate(store, &transaction.targets, &mut stages, injection) {
        compensation_errors.push(error);
    }
    if !compensation_errors.is_empty() {
        transaction.state = TransactionState::Committing;
        transaction.error = combined_message(&cause, &compensation_errors);
        transaction.updated_at = CreatedAt::now();
        if let Err(error) = persist_manifest(store, transaction, TransactionPhase::PersistAborted) {
            compensation_errors.push(error);
        }
        return Err(TransactionError::CompensationPending {
            cause: Box::new(cause),
            compensation: Box::new(TransactionError::Compensation(compensation_errors)),
            stages,
        });
    }

    transaction.state = TransactionState::Aborted;
    transaction.error = cause.to_string();
    transaction.updated_at = CreatedAt::now();
    if let Err(error) = persist_manifest(store, transaction, TransactionPhase::PersistAborted) {
        return Err(TransactionError::CompensationPending {
            cause: Box::new(cause),
            compensation: Box::new(error),
            stages,
        });
    }
    Err(cause.with_stages(stages))
}

fn combined_message(cause: &TransactionError, errors: &[TransactionError]) -> String {
    let mut messages = vec![cause.to_string()];
    messages.extend(errors.iter().map(ToString::to_string));
    messages.join("; ")
}
