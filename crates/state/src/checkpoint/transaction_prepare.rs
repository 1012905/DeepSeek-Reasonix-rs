use std::io;
use std::path::{Path, PathBuf};

use reasonix_core::fileencoding::{self, Kind};

use super::atomic_json::write_json_atomic;
use super::blob::{BlobError, digest};
use super::path_capture::{Fingerprint, fingerprint_path, safe_path};
use super::secure_path::{self, SecureError};
use super::store::Store;
use super::transaction::{TransactionError, TransactionInjection, TransactionPhase};
use super::types::{
    Checkpoint, FileRevision, RewindPlan, RewindScope, SCHEMA_V2, TransactionManifest,
    TransactionState, TransactionTarget, transaction_dir,
};

pub(super) fn prepare(
    store: &Store,
    plan: &RewindPlan,
    conversation_forward: &[u8],
    injection: Option<&TransactionInjection>,
) -> Result<TransactionManifest, TransactionError> {
    let created_at = crate::sessionstore::eventlog::record::CreatedAt::now();
    let mut transaction = TransactionManifest {
        schema_version: SCHEMA_V2,
        id: new_transaction_id(plan, created_at),
        session_id: store.state().session_id.clone(),
        workspace_root: store.root.to_string_lossy().into_owned(),
        state: TransactionState::Prepared,
        kind: "rewind".to_owned(),
        turn: plan.turn,
        scope: plan.scope,
        created_at,
        updated_at: created_at,
        session_revision: plan.session_revision,
        workspace_token: plan.workspace_token.clone(),
        coverage: plan.coverage,
        coverage_gaps: plan.coverage_gaps.clone(),
        conversation_forward: conversation_forward.to_vec(),
        boundary_index: plan.boundary_index,
        has_boundary: plan.has_boundary,
        truncate_from: plan.turn,
        ..TransactionManifest::default()
    };

    let prepared = build_targets(store, plan, &mut transaction);
    if matches!(plan.scope, RewindScope::Conversation | RewindScope::Both) {
        match checkpoint_backup(store, plan.turn) {
            Ok(backup) => transaction.checkpoint_backup = backup,
            Err(error) => {
                let error = TransactionError::at(TransactionPhase::Prepare, error);
                return abort_prepare(store, transaction, error);
            }
        }
    }
    if let Err(error) = prepared {
        return abort_prepare(store, transaction, error);
    }
    if injection.is_some_and(|value| value.fails(TransactionPhase::PersistPrepared)) {
        return abort_prepare(
            store,
            transaction,
            TransactionError::injected(TransactionPhase::PersistPrepared),
        );
    }
    persist_manifest(store, &transaction, TransactionPhase::PersistPrepared)?;
    Ok(transaction)
}

fn build_targets(
    store: &Store,
    plan: &RewindPlan,
    transaction: &mut TransactionManifest,
) -> Result<(), TransactionError> {
    if !matches!(plan.scope, RewindScope::Code | RewindScope::Both) {
        return Ok(());
    }
    for (index, (path, revision)) in store.earliest_revisions(plan.turn).into_iter().enumerate() {
        let absolute = safe_path(&store.root, Path::new(&path))
            .map_err(|error| TransactionError::at(TransactionPhase::Prepare, error.into()))?;
        let forward = fingerprint_path(&store.root, &absolute).map_err(|error| {
            TransactionError::at(TransactionPhase::Prepare, TransactionError::Capture(error))
        })?;
        let (publish_tmp, backup_path) = sibling_paths(&absolute, &transaction.id, index)?;
        let restore_data = revision
            .existed
            .then(|| restore_bytes(store, &revision, &forward, &path))
            .transpose()?;
        let mut target = TransactionTarget {
            path,
            abs_path: absolute.to_string_lossy().into_owned(),
            restore_existed: revision.existed,
            restore_mode: revision.mode,
            restore_sha: revision.sha256.clone(),
            restore_blob: revision.blob_ref.clone(),
            restore_encoding: revision.encoding,
            forward_existed: forward.existed,
            forward_mode: forward.mode,
            forward_sha: forward.sha256,
            backup_path: backup_path.to_string_lossy().into_owned(),
            ..TransactionTarget::default()
        };
        if revision.existed {
            target.action = "write".to_owned();
            let data = restore_data.expect("restore data for existing revision");
            let mode = if revision.mode == 0 {
                0o644
            } else {
                revision.mode
            };
            if target.restore_blob.is_empty() {
                if let Some(blobs) = &store.blobs {
                    target.restore_blob = blobs.put(&data).map_err(|error| {
                        TransactionError::at(TransactionPhase::Prepare, error.into())
                    })?;
                } else {
                    target.restore_inline = data.clone();
                }
            }
            target.publish_tmp = publish_tmp.to_string_lossy().into_owned();
            secure_path::write_new_file(&store.root, &publish_tmp, &data, mode)
                .map_err(|error| TransactionError::at(TransactionPhase::Prepare, error.into()))?;
        } else {
            target.action = "delete".to_owned();
        }
        if forward.existed {
            let data = forward.content.as_deref().unwrap_or_default();
            if let Some(blobs) = &store.blobs {
                target.forward_blob = blobs.put(data).map_err(|error| {
                    TransactionError::at(TransactionPhase::Prepare, error.into())
                })?;
            } else {
                target.forward_inline = data.to_vec();
            }
        }
        transaction.targets.push(target);
    }
    Ok(())
}

fn restore_bytes(
    store: &Store,
    revision: &FileRevision,
    forward: &Fingerprint,
    path: &str,
) -> Result<Vec<u8>, TransactionError> {
    if !revision.blob_ref.is_empty() {
        let blobs = store
            .blobs
            .as_ref()
            .ok_or_else(|| TransactionError::MissingPayload {
                path: path.to_owned(),
                reference: revision.blob_ref.clone(),
            })?;
        return match blobs.get(&revision.blob_ref) {
            Ok(data) => Ok(data),
            Err(BlobError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                Err(TransactionError::MissingPayload {
                    path: path.to_owned(),
                    reference: revision.blob_ref.clone(),
                })
            }
            Err(error) => Err(TransactionError::at(
                TransactionPhase::Prepare,
                error.into(),
            )),
        };
    }
    let Some(text) = revision.content.as_deref() else {
        return Err(TransactionError::MissingPayload {
            path: path.to_owned(),
            reference: revision.sha256.clone(),
        });
    };
    let kind = revision
        .encoding
        .or(Some(forward.encoding))
        .unwrap_or(Kind::Utf8);
    fileencoding::encode(text, kind).map_err(|error| {
        TransactionError::at(TransactionPhase::Prepare, TransactionError::Encoding(error))
    })
}

fn checkpoint_backup(store: &Store, from_turn: i64) -> Result<Vec<u8>, TransactionError> {
    let checkpoints: Vec<Checkpoint> = store
        .state()
        .all()
        .into_iter()
        .filter(|checkpoint| checkpoint.turn >= from_turn)
        .collect();
    serde_json::to_vec(&checkpoints).map_err(|error| {
        TransactionError::at(
            TransactionPhase::Prepare,
            SecureError::JsonEncode {
                path: transaction_dir(store.dir.as_deref().unwrap_or_else(|| Path::new("")))
                    .join("checkpoint-backup.json"),
                source: error,
            }
            .into(),
        )
    })
}

fn abort_prepare(
    store: &Store,
    mut transaction: TransactionManifest,
    cause: TransactionError,
) -> Result<TransactionManifest, TransactionError> {
    let _ = cleanup_temps(store, &transaction.targets);
    transaction.state = TransactionState::Aborted;
    transaction.error = cause.to_string();
    transaction.updated_at = crate::sessionstore::eventlog::record::CreatedAt::now();
    if let Err(error) = persist_manifest(store, &transaction, TransactionPhase::PersistAborted) {
        return Err(TransactionError::CompensationPending {
            cause: Box::new(cause),
            compensation: Box::new(error),
            stages: Vec::new(),
        });
    }
    Err(cause)
}

pub(super) fn sibling_paths(
    absolute: &Path,
    transaction_id: &str,
    index: usize,
) -> Result<(PathBuf, PathBuf), TransactionError> {
    let name = absolute.file_name().ok_or_else(|| {
        TransactionError::at(TransactionPhase::Prepare, SecureError::WorkspaceRoot.into())
    })?;
    let parent = absolute.parent().unwrap_or_else(|| Path::new("."));
    let prefix = format!(
        ".{}.reasonix-{transaction_id}-{index}",
        name.to_string_lossy()
    );
    Ok((
        parent.join(format!("{prefix}.tmp")),
        parent.join(format!("{prefix}.bak")),
    ))
}

pub(super) fn new_transaction_id(
    plan: &RewindPlan,
    created_at: crate::sessionstore::eventlog::record::CreatedAt,
) -> String {
    let timestamp = created_at.get().timestamp_nanos_opt().unwrap_or_default();
    let suffix = digest(format!("{}-{timestamp}", plan.plan_id).as_bytes());
    format!("tx-{timestamp}-{}", &suffix[..8])
}

pub(super) fn persist_manifest(
    store: &Store,
    transaction: &TransactionManifest,
    phase: TransactionPhase,
) -> Result<(), TransactionError> {
    let Some(dir) = store.dir.as_deref() else {
        return Ok(());
    };
    let path = transaction_dir(dir).join(format!("{}.json", transaction.id));
    write_json_atomic(&path, transaction).map_err(|error| TransactionError::at(phase, error.into()))
}

pub(super) fn cleanup_temps(
    store: &Store,
    targets: &[TransactionTarget],
) -> Result<(), TransactionError> {
    let mut errors = Vec::new();
    for target in targets {
        if target.publish_tmp.is_empty() {
            continue;
        }
        if let Err(error) = secure_path::remove(&store.root, Path::new(&target.publish_tmp))
            && !error.is_not_found()
        {
            errors.push(TransactionError::at(
                TransactionPhase::Cleanup,
                error.into(),
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(TransactionError::Compensation(errors))
    }
}
