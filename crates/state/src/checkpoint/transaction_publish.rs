use std::path::Path;

use crate::sessionstore::eventlog::record::CreatedAt;

use super::secure_path;
use super::store::Store;
use super::transaction::{
    ConversationApplier, ConversationError, ConversationPhase, TransactionError,
    TransactionInjection, TransactionPhase,
};
use super::transaction_compensate;
use super::transaction_prepare::persist_manifest;
use super::types::{
    FileStage, RewindResult, RewindScope, TransactionManifest, TransactionState, TransactionTarget,
};

pub(super) fn commit(
    store: &Store,
    transaction: &mut TransactionManifest,
    applier: Option<&dyn ConversationApplier>,
    injection: Option<&TransactionInjection>,
) -> Result<RewindResult, TransactionError> {
    let mut transaction = store.track_undo_transaction(transaction)?;
    let result = publish(store, &mut transaction, applier, injection);
    drop(transaction);
    result
}

fn publish(
    store: &Store,
    transaction: &mut TransactionManifest,
    applier: Option<&dyn ConversationApplier>,
    injection: Option<&TransactionInjection>,
) -> Result<RewindResult, TransactionError> {
    transaction.state = TransactionState::Committing;
    transaction.updated_at = CreatedAt::now();
    if injection.is_some_and(|value| value.fails(TransactionPhase::PersistCommitting)) {
        let cause = TransactionError::injected(TransactionPhase::PersistCommitting);
        transaction_compensate::fail(
            store,
            transaction,
            Vec::new(),
            cause,
            applier,
            false,
            injection,
        )?;
        unreachable!("clean compensation still returns the transaction cause")
    }
    if let Err(cause) = persist_manifest(store, transaction, TransactionPhase::PersistCommitting) {
        return fail(
            store,
            transaction,
            Vec::new(),
            cause,
            applier,
            false,
            injection,
        );
    }

    let mut result = RewindResult {
        transaction_id: transaction.id.clone(),
        undo_available: false,
        coverage: transaction.coverage,
        coverage_gaps: transaction.coverage_gaps.clone(),
        ..RewindResult::default()
    };
    let mut stages = Vec::with_capacity(transaction.targets.len());
    for (files_done, index) in (0..transaction.targets.len()).enumerate() {
        let target = transaction.targets[index].clone();
        let phase = if target.action == "delete" {
            TransactionPhase::DeleteFile
        } else {
            TransactionPhase::PublishFile
        };
        let mut stage = FileStage {
            path: target.path.clone(),
            phase: "commit".to_owned(),
            action: target.action.clone(),
            ..FileStage::default()
        };
        if injection.is_some_and(|value| value.fails_after_files(phase, files_done)) {
            let cause = TransactionError::injected(phase);
            stage.error = cause.to_string();
            stages.push(stage);
            return fail(store, transaction, stages, cause, applier, false, injection);
        }

        let mut candidate = transaction.clone();
        candidate.targets[index].published = true;
        candidate.updated_at = CreatedAt::now();
        if injection.is_some_and(|value| value.fails(TransactionPhase::PersistPublishedIntent)) {
            let cause = TransactionError::injected(TransactionPhase::PersistPublishedIntent);
            stage.error = cause.to_string();
            stages.push(stage);
            return fail(store, transaction, stages, cause, applier, false, injection);
        }
        if let Err(cause) =
            persist_manifest(store, &candidate, TransactionPhase::PersistPublishedIntent)
        {
            stage.error = cause.to_string();
            stages.push(stage);
            return fail(store, transaction, stages, cause, applier, false, injection);
        }
        *transaction = candidate;
        if injection.is_some_and(|value| value.crashes(TransactionPhase::CrashAfterPublishedIntent))
        {
            return Err(TransactionError::InjectedCrash {
                phase: TransactionPhase::CrashAfterPublishedIntent,
            });
        }

        if let Err(cause) = publish_target(store, &target) {
            stage.error = cause.to_string();
            stages.push(stage);
            return fail(store, transaction, stages, cause, applier, false, injection);
        }
        if injection
            .is_some_and(|value| value.crashes(TransactionPhase::CrashAfterPublishBeforeProgress))
        {
            stage.phase = "done".to_owned();
            stages.push(stage);
            return Err(TransactionError::InjectedCrash {
                phase: TransactionPhase::CrashAfterPublishBeforeProgress,
            });
        }
        transaction.updated_at = CreatedAt::now();
        if injection.is_some_and(|value| value.fails(TransactionPhase::PersistProgress)) {
            let cause = TransactionError::injected(TransactionPhase::PersistProgress);
            stage.error = cause.to_string();
            stages.push(stage);
            return fail(store, transaction, stages, cause, applier, false, injection);
        }
        if let Err(cause) = persist_manifest(store, transaction, TransactionPhase::PersistProgress)
        {
            stage.error = cause.to_string();
            stages.push(stage);
            return fail(store, transaction, stages, cause, applier, false, injection);
        }
        stage.phase = "done".to_owned();
        stages.push(stage);
        if target.action == "write" {
            result.written.push(target.path);
        } else {
            result.deleted.push(target.path);
        }
    }
    if injection.is_some_and(|value| value.fails(TransactionPhase::PersistProgress)) {
        let cause = TransactionError::injected(TransactionPhase::PersistProgress);
        return fail(store, transaction, stages, cause, applier, false, injection);
    }
    if let Err(cause) = persist_manifest(store, transaction, TransactionPhase::PersistProgress) {
        return fail(store, transaction, stages, cause, applier, false, injection);
    }

    if matches!(
        transaction.scope,
        RewindScope::Conversation | RewindScope::Both
    ) {
        if injection.is_some_and(|value| value.fails(TransactionPhase::BeforeConversation)) {
            let cause = TransactionError::injected(TransactionPhase::BeforeConversation);
            return fail(store, transaction, stages, cause, applier, false, injection);
        }
        if transaction.has_boundary
            && let Some(applier) = applier
        {
            if let Err(source) = applier.apply_conversation_truncate(
                transaction.boundary_index,
                &transaction.conversation_forward,
            ) {
                let cause = TransactionError::Conversation {
                    phase: ConversationPhase::ApplyConversationTruncate,
                    source,
                };
                return fail(
                    store,
                    transaction,
                    stages,
                    cause,
                    Some(applier),
                    true,
                    injection,
                );
            }
            result.conversation_ok = true;
        }
        if injection.is_some_and(|value| value.fails(TransactionPhase::BeforeCheckpointTruncate)) {
            let cause = TransactionError::injected(TransactionPhase::BeforeCheckpointTruncate);
            return fail(store, transaction, stages, cause, applier, true, injection);
        }
        if let Some(applier) = applier {
            if let Err(source) = applier.truncate_checkpoints(transaction.truncate_from) {
                let cause = TransactionError::Conversation {
                    phase: ConversationPhase::TruncateCheckpoints,
                    source,
                };
                return fail(
                    store,
                    transaction,
                    stages,
                    cause,
                    Some(applier),
                    true,
                    injection,
                );
            }
        } else if let Err(source) = store.truncate_from(transaction.truncate_from) {
            let cause = TransactionError::Conversation {
                phase: ConversationPhase::TruncateCheckpoints,
                source: ConversationError::external(ConversationPhase::TruncateCheckpoints, source),
            };
            return fail(store, transaction, stages, cause, None, true, injection);
        }
    }

    if injection.is_some_and(|value| value.fails(TransactionPhase::SyncRestoredOwnership)) {
        let cause = TransactionError::injected(TransactionPhase::SyncRestoredOwnership);
        return fail(store, transaction, stages, cause, applier, true, injection);
    }
    if let Err(error) = store.sync_restored_ownership(&transaction.targets) {
        let cause = TransactionError::at(TransactionPhase::SyncRestoredOwnership, error.into());
        return fail(store, transaction, stages, cause, applier, true, injection);
    }
    if injection
        .is_some_and(|value| value.crashes(TransactionPhase::CrashAfterConversationBeforeFinalize))
    {
        return Err(TransactionError::InjectedCrash {
            phase: TransactionPhase::CrashAfterConversationBeforeFinalize,
        });
    }
    if injection.is_some_and(|value| value.fails(TransactionPhase::Finalize)) {
        let cause = TransactionError::injected(TransactionPhase::Finalize);
        return fail(store, transaction, stages, cause, applier, true, injection);
    }

    transaction.state = TransactionState::Committed;
    transaction.updated_at = CreatedAt::now();
    result.ok = true;
    if injection.is_some_and(|value| value.fails(TransactionPhase::PersistCommitted)) {
        let cause = TransactionError::injected(TransactionPhase::PersistCommitted);
        return fail(store, transaction, stages, cause, applier, true, injection);
    }
    if let Err(error) = persist_manifest(store, transaction, TransactionPhase::PersistCommitted) {
        return fail(store, transaction, stages, error, applier, true, injection);
    }
    cleanup_committed(store, &transaction.targets);
    store.publish_undo_offer(transaction);
    result.files = stages;
    Ok(result)
}

fn fail(
    store: &Store,
    transaction: &mut TransactionManifest,
    stages: Vec<FileStage>,
    cause: TransactionError,
    applier: Option<&dyn ConversationApplier>,
    restore_state: bool,
    injection: Option<&TransactionInjection>,
) -> Result<RewindResult, TransactionError> {
    transaction_compensate::fail(
        store,
        transaction,
        stages,
        cause,
        applier,
        restore_state,
        injection,
    )?;
    unreachable!("clean compensation still returns the transaction cause")
}

pub(super) fn publish_target(
    store: &Store,
    target: &TransactionTarget,
) -> Result<(), TransactionError> {
    if target.backup_path.is_empty() {
        return Err(TransactionError::MissingBackupPath {
            path: target.path.clone(),
        });
    }
    let backup = Path::new(&target.backup_path);
    if secure_path::path_exists(store.root.as_path(), backup)
        .map_err(|error| TransactionError::at(TransactionPhase::PublishFile, error.into()))?
    {
        return Err(TransactionError::BackupAlreadyExists {
            path: target.path.clone(),
        });
    }
    let absolute = Path::new(&target.abs_path);
    let target_existed = secure_path::path_exists(store.root.as_path(), absolute)
        .map_err(|error| TransactionError::at(TransactionPhase::PublishFile, error.into()))?;
    if target_existed {
        secure_path::rename(store.root.as_path(), absolute, backup)
            .map_err(|error| TransactionError::at(TransactionPhase::PublishFile, error.into()))?;
    }
    if target.action == "delete" {
        return Ok(());
    }
    if target.publish_tmp.is_empty() {
        return Err(TransactionError::MissingPublishTemp {
            path: target.path.clone(),
        });
    }
    let temp = Path::new(&target.publish_tmp);
    if let Err(error) = secure_path::rename(store.root.as_path(), temp, absolute) {
        let source = TransactionError::at(TransactionPhase::PublishFile, error.into());
        if target_existed
            && let Err(restore) = secure_path::rename(store.root.as_path(), backup, absolute)
        {
            return Err(TransactionError::Compensation(vec![
                source,
                TransactionError::at(TransactionPhase::CompensateFile, restore.into()),
            ]));
        }
        return Err(source);
    }
    if target.restore_mode != 0 {
        secure_path::chmod(store.root.as_path(), absolute, target.restore_mode)
            .map_err(|error| TransactionError::at(TransactionPhase::PublishFile, error.into()))?;
    }
    Ok(())
}

fn cleanup_committed(store: &Store, targets: &[TransactionTarget]) {
    for target in targets {
        if target.backup_path.is_empty() {
            continue;
        }
        let _ = secure_path::remove(store.root.as_path(), Path::new(&target.backup_path));
    }
}
