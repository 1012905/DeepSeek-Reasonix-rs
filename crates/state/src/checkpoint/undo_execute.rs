use std::path::Path;

use crate::sessionstore::eventlog::record::CreatedAt;
use tracing::warn;

use super::path_capture::{compare_identity, fingerprint_path};
use super::secure_path;
use super::store::Store;
use super::transaction::{
    ConversationApplier, ConversationError, ConversationPhase, TransactionError,
    TransactionInjection, TransactionPhase,
};
use super::transaction_compensate;
use super::transaction_prepare::{
    load_restore_payload, new_transaction_id, persist_manifest, sibling_paths,
};
use super::transaction_publish::{cleanup_committed, publish_files, start_publish};
use super::types::{
    CONFLICT_MANUAL_EDIT, Coverage, FileStage, RewindConflict, RewindPlan, RewindResult,
    RewindScope, SCHEMA_V2, TransactionManifest, TransactionState, TransactionTarget,
};

impl Store {
    pub fn undo_rewind(
        &self,
        transaction_id: &str,
        applier: Option<&dyn ConversationApplier>,
        injection: Option<&TransactionInjection>,
    ) -> Result<RewindResult, TransactionError> {
        let mut original = self.undo_offer(transaction_id)?;
        let _permit = self
            .barrier()
            .try_enter_exclusive()
            .ok_or(TransactionError::BarrierBusy)?;
        if !self.state().active_writers.is_empty() {
            return Err(TransactionError::ActiveWriters);
        }
        let conflicts = undo_file_conflicts(self, &original)?;
        if !conflicts.is_empty() {
            return Err(TransactionError::UndoFileConflicts(conflicts));
        }

        let mut undo = inverse_manifest(&original)?;
        persist_prepared_undo(self, &mut undo, injection)?;
        let mut cleanup_copy = undo.clone();
        let tracking = self.track_undo_offer(transaction_id, &mut undo);
        let mut tracked = match tracking {
            Ok(tracked) => tracked,
            Err(cause) => {
                transaction_compensate::fail(
                    self,
                    &mut cleanup_copy,
                    Vec::new(),
                    cause,
                    None,
                    false,
                    injection,
                )?;
                unreachable!("clean compensation still returns the transaction cause")
            }
        };
        let result = stage_and_publish_undo(self, &mut tracked, &mut original, applier, injection);
        drop(tracked);
        result
    }
}

fn undo_file_conflicts(
    store: &Store,
    original: &TransactionManifest,
) -> Result<Vec<RewindConflict>, TransactionError> {
    for target in &original.targets {
        let current = fingerprint_path(&store.root, Path::new(&target.abs_path))
            .map_err(TransactionError::Capture)?;
        if target.action == "delete" {
            if current.existed {
                return Ok(vec![RewindConflict {
                    path: target.path.clone(),
                    reason: CONFLICT_MANUAL_EDIT.to_owned(),
                    current_sha: current.sha256,
                    ..RewindConflict::default()
                }]);
            }
            continue;
        }
        if let Some(reason) = compare_identity(
            &current,
            &target.restore_sha,
            Some(target.restore_existed),
            target.restore_mode,
        ) {
            return Ok(vec![RewindConflict {
                path: target.path.clone(),
                reason: reason.to_owned(),
                last_owned_sha: target.restore_sha.clone(),
                current_sha: current.sha256,
                checkpoint_mode: target.restore_mode,
                current_mode: current.mode,
                ..RewindConflict::default()
            }]);
        }
    }
    Ok(Vec::new())
}

fn persist_prepared_undo(
    store: &Store,
    undo: &mut TransactionManifest,
    injection: Option<&TransactionInjection>,
) -> Result<(), TransactionError> {
    if injection.is_some_and(|value| value.fails(TransactionPhase::PersistPrepared)) {
        return Err(TransactionError::injected(
            TransactionPhase::PersistPrepared,
        ));
    }
    persist_manifest(store, undo, TransactionPhase::PersistPrepared)?;
    if injection.is_some_and(|value| value.crashes(TransactionPhase::CrashAfterPrepared)) {
        return Err(TransactionError::InjectedCrash {
            phase: TransactionPhase::CrashAfterPrepared,
        });
    }
    Ok(())
}

fn stage_and_publish_undo(
    store: &Store,
    undo: &mut TransactionManifest,
    original: &mut TransactionManifest,
    applier: Option<&dyn ConversationApplier>,
    injection: Option<&TransactionInjection>,
) -> Result<RewindResult, TransactionError> {
    stage_undo(store, undo, injection)?;
    publish_undo(store, undo, original, applier, injection)
}

fn stage_undo(
    store: &Store,
    undo: &mut TransactionManifest,
    injection: Option<&TransactionInjection>,
) -> Result<(), TransactionError> {
    for target in &undo.targets {
        if target.action != "write" {
            continue;
        }
        let result = (|| {
            let data = load_restore_payload(store, target)?;
            let mode = if target.restore_mode == 0 {
                0o644
            } else {
                target.restore_mode
            };
            secure_path::write_new_file(&store.root, Path::new(&target.publish_tmp), &data, mode)
                .map_err(|error| TransactionError::at(TransactionPhase::Prepare, error.into()))
        })();
        if let Err(cause) = result {
            return abort_undo(store, undo, Vec::new(), cause, injection);
        }
    }
    undo.updated_at = CreatedAt::now();
    if let Err(cause) = persist_manifest(store, undo, TransactionPhase::PersistPrepared) {
        return abort_undo(store, undo, Vec::new(), cause, injection);
    }
    Ok(())
}

fn inverse_manifest(
    original: &TransactionManifest,
) -> Result<TransactionManifest, TransactionError> {
    let created_at = CreatedAt::now();
    let plan = RewindPlan {
        plan_id: format!("undo-{}", original.id),
        ..RewindPlan::default()
    };
    let mut undo = TransactionManifest {
        schema_version: SCHEMA_V2,
        id: new_transaction_id(&plan, created_at),
        session_id: original.session_id.clone(),
        workspace_root: original.workspace_root.clone(),
        state: TransactionState::Prepared,
        kind: "undo".to_owned(),
        turn: original.turn,
        scope: original.scope,
        created_at,
        updated_at: created_at,
        session_revision: original.session_revision,
        conversation_forward: original.conversation_forward.clone(),
        boundary_index: original.boundary_index,
        has_boundary: original.has_boundary,
        truncate_from: original.truncate_from,
        checkpoint_backup: original.checkpoint_backup.clone(),
        parent_transaction: original.id.clone(),
        ..TransactionManifest::default()
    };
    for target in &original.targets {
        let (publish_tmp, backup_path) =
            sibling_paths(Path::new(&target.abs_path), &undo.id, undo.targets.len())?;
        let action = if target.forward_existed {
            "write"
        } else {
            "delete"
        };
        undo.targets.push(TransactionTarget {
            path: target.path.clone(),
            abs_path: target.abs_path.clone(),
            restore_existed: target.forward_existed,
            restore_mode: target.forward_mode,
            restore_sha: target.forward_sha.clone(),
            restore_blob: target.forward_blob.clone(),
            restore_inline: target.forward_inline.clone(),
            restore_encoding: None,
            forward_existed: target.restore_existed,
            forward_mode: target.restore_mode,
            forward_sha: target.restore_sha.clone(),
            forward_blob: target.restore_blob.clone(),
            forward_inline: target.restore_inline.clone(),
            publish_tmp: if action == "write" {
                publish_tmp.to_string_lossy().into_owned()
            } else {
                String::new()
            },
            backup_path: backup_path.to_string_lossy().into_owned(),
            published: false,
            action: action.to_owned(),
        });
    }
    Ok(undo)
}

fn publish_undo(
    store: &Store,
    undo: &mut TransactionManifest,
    original: &mut TransactionManifest,
    applier: Option<&dyn ConversationApplier>,
    injection: Option<&TransactionInjection>,
) -> Result<RewindResult, TransactionError> {
    if let Err(cause) = start_publish(store, undo, injection) {
        return fail_undo(store, undo, Vec::new(), cause, None, injection);
    }
    let mut stages = Vec::with_capacity(undo.targets.len());
    let mut result = match publish_files(store, undo, injection, &mut stages) {
        Ok(result) => result,
        Err(cause @ TransactionError::InjectedCrash { .. }) => return Err(cause),
        Err(cause) => return fail_undo(store, undo, stages, cause, None, injection),
    };
    result.coverage = Coverage::Complete;
    result.coverage_gaps.clear();

    if let Some(applier) = applier
        && !original.conversation_forward.is_empty()
    {
        if let Err(source) = applier.restore_conversation(&original.conversation_forward) {
            let cause = TransactionError::Conversation {
                phase: ConversationPhase::RestoreConversation,
                source,
            };
            let compensation = restore_original_rewind(store, original, Some(applier)).err();
            return fail_undo(store, undo, stages, cause, compensation, injection);
        }
        result.conversation_ok = true;
    }
    if !original.checkpoint_backup.is_empty() {
        let restored = if let Some(applier) = applier {
            applier
                .restore_checkpoints(&original.checkpoint_backup)
                .map_err(|source| TransactionError::Conversation {
                    phase: ConversationPhase::RestoreCheckpoints,
                    source,
                })
        } else {
            store
                .restore_checkpoint_backup(&original.checkpoint_backup)
                .map_err(|source| TransactionError::Conversation {
                    phase: ConversationPhase::RestoreCheckpoints,
                    source: ConversationError::external(
                        ConversationPhase::RestoreCheckpoints,
                        source,
                    ),
                })
        };
        if let Err(cause) = restored {
            let compensation = restore_original_rewind(store, original, applier).err();
            return fail_undo(store, undo, stages, cause, compensation, injection);
        }
    }
    finish_undo(store, undo, original, applier, result, stages, injection)
}

fn restore_original_rewind(
    _store: &Store,
    original: &TransactionManifest,
    applier: Option<&dyn ConversationApplier>,
) -> Result<(), TransactionError> {
    let Some(applier) = applier else {
        return Ok(());
    };
    if !matches!(
        original.scope,
        RewindScope::Conversation | RewindScope::Both
    ) {
        return Ok(());
    }
    let mut errors = Vec::new();
    if original.has_boundary
        && let Err(source) = applier
            .apply_conversation_truncate(original.boundary_index, &original.conversation_forward)
    {
        errors.push(TransactionError::Conversation {
            phase: ConversationPhase::ApplyConversationTruncate,
            source,
        });
    }
    if let Err(source) = applier.truncate_checkpoints(original.truncate_from) {
        errors.push(TransactionError::Conversation {
            phase: ConversationPhase::TruncateCheckpoints,
            source,
        });
    }
    match errors.len() {
        0 => Ok(()),
        1 => Err(errors.pop().expect("single compensation error")),
        _ => Err(TransactionError::Compensation(errors)),
    }
}

fn finish_undo(
    store: &Store,
    undo: &mut TransactionManifest,
    original: &mut TransactionManifest,
    applier: Option<&dyn ConversationApplier>,
    mut result: RewindResult,
    stages: Vec<FileStage>,
    injection: Option<&TransactionInjection>,
) -> Result<RewindResult, TransactionError> {
    if injection.is_some_and(|value| value.fails(TransactionPhase::SyncRestoredOwnership)) {
        let cause = TransactionError::injected(TransactionPhase::SyncRestoredOwnership);
        let compensation = restore_original_rewind(store, original, applier).err();
        return fail_undo(store, undo, stages, cause, compensation, injection);
    }
    if let Err(error) = store.sync_restored_ownership(&undo.targets) {
        let cause = TransactionError::at(TransactionPhase::SyncRestoredOwnership, error.into());
        let compensation = restore_original_rewind(store, original, applier).err();
        return fail_undo(store, undo, stages, cause, compensation, injection);
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
        let compensation = restore_original_rewind(store, original, applier).err();
        return fail_undo(store, undo, stages, cause, compensation, injection);
    }

    undo.state = TransactionState::Committed;
    undo.updated_at = CreatedAt::now();
    if injection.is_some_and(|value| value.fails(TransactionPhase::PersistCommitted)) {
        let cause = TransactionError::injected(TransactionPhase::PersistCommitted);
        let compensation = restore_original_rewind(store, original, applier).err();
        return fail_undo(store, undo, stages, cause, compensation, injection);
    }
    if let Err(cause) = persist_manifest(store, undo, TransactionPhase::PersistCommitted) {
        let compensation = restore_original_rewind(store, original, applier).err();
        return fail_undo(store, undo, stages, cause, compensation, injection);
    }
    cleanup_committed(store, &undo.targets);

    original.state = TransactionState::Undone;
    original.updated_at = CreatedAt::now();
    if !injection.is_some_and(|value| value.fails(TransactionPhase::PersistParentUndone))
        && let Err(error) = persist_manifest(store, original, TransactionPhase::PersistParentUndone)
    {
        // The committed inverse names its parent, so startup still suppresses the stale offer.
        warn!(transaction_id = %original.id, %error, "checkpoint: persist parent as undone");
    }
    store.retire_undo_offer(&original.id);
    result.ok = true;
    result.undo_available = false;
    result.files = stages;
    Ok(result)
}

fn abort_undo(
    store: &Store,
    undo: &mut TransactionManifest,
    stages: Vec<FileStage>,
    cause: TransactionError,
    injection: Option<&TransactionInjection>,
) -> Result<(), TransactionError> {
    fail_undo(store, undo, stages, cause, None, injection)?;
    unreachable!("clean compensation still returns the transaction cause")
}

fn fail_undo(
    store: &Store,
    undo: &mut TransactionManifest,
    stages: Vec<FileStage>,
    cause: TransactionError,
    state_compensation: Option<TransactionError>,
    injection: Option<&TransactionInjection>,
) -> Result<RewindResult, TransactionError> {
    transaction_compensate::fail_after_state_compensation(
        store,
        undo,
        stages,
        cause,
        state_compensation,
        injection,
    )?;
    unreachable!("clean compensation still returns the transaction cause")
}
