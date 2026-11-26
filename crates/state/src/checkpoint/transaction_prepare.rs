use std::io;
use std::path::{Path, PathBuf};

use reasonix_contract::gojson;
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
    with_checkpoint_backup: bool,
    injection: Option<&TransactionInjection>,
) -> Result<TransactionManifest, TransactionError> {
    let mut transaction = new_manifest(store, plan, "rewind");
    transaction.conversation_forward = conversation_forward.to_vec();
    let result = (|| {
        build_targets(store, plan, &mut transaction)?;
        if with_checkpoint_backup
            && matches!(plan.scope, RewindScope::Conversation | RewindScope::Both)
        {
            transaction.checkpoint_backup = checkpoint_backup(store, plan.turn)?;
        }
        if injection.is_some_and(|value| value.fails(TransactionPhase::PersistPrepared)) {
            return Err(TransactionError::injected(
                TransactionPhase::PersistPrepared,
            ));
        }
        persist_manifest(store, &transaction, TransactionPhase::PersistPrepared)
    })();
    if let Err(cause) = result {
        return abort_prepare(store, transaction, cause);
    }
    Ok(transaction)
}

pub(super) fn prepare_file_revert(
    store: &Store,
    plan: &RewindPlan,
    revision: &FileRevision,
    injection: Option<&TransactionInjection>,
) -> Result<TransactionManifest, TransactionError> {
    let mut transaction = new_manifest(store, plan, "file_revert");
    transaction.scope = RewindScope::Code;
    transaction.path = plan.path.clone();
    transaction.truncate_from = 0;
    let result = (|| {
        let target = build_target(store, &transaction, &revision.path, revision, 0)?;
        transaction.targets.push(target);
        if injection.is_some_and(|value| value.fails(TransactionPhase::PersistPrepared)) {
            return Err(TransactionError::injected(
                TransactionPhase::PersistPrepared,
            ));
        }
        persist_manifest(store, &transaction, TransactionPhase::PersistPrepared)
    })();
    if let Err(cause) = result {
        let _ = cleanup_temps(store, &transaction.targets);
        return Err(cause);
    }
    Ok(transaction)
}

fn new_manifest(store: &Store, plan: &RewindPlan, kind: &str) -> TransactionManifest {
    let created_at = crate::sessionstore::eventlog::record::CreatedAt::now();
    TransactionManifest {
        schema_version: SCHEMA_V2,
        id: new_transaction_id(plan, created_at),
        session_id: store.state().session_id.clone(),
        workspace_root: store.root.to_string_lossy().into_owned(),
        state: TransactionState::Prepared,
        kind: kind.to_owned(),
        turn: plan.turn,
        scope: plan.scope,
        created_at,
        updated_at: created_at,
        session_revision: plan.session_revision,
        workspace_token: plan.workspace_token.clone(),
        coverage: plan.coverage,
        coverage_gaps: plan.coverage_gaps.clone(),
        boundary_index: plan.boundary_index,
        has_boundary: plan.has_boundary,
        truncate_from: plan.turn,
        ..TransactionManifest::default()
    }
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
        let target = build_target(store, transaction, &path, &revision, index)?;
        transaction.targets.push(target);
    }
    Ok(())
}

fn build_target(
    store: &Store,
    transaction: &TransactionManifest,
    path: &str,
    revision: &FileRevision,
    index: usize,
) -> Result<TransactionTarget, TransactionError> {
    let absolute = safe_path(&store.root, Path::new(path))
        .map_err(|error| TransactionError::at(TransactionPhase::Prepare, error.into()))?;
    let forward = fingerprint_path(&store.root, &absolute).map_err(|error| {
        TransactionError::at(TransactionPhase::Prepare, TransactionError::Capture(error))
    })?;
    let (publish_tmp, backup_path) = sibling_paths(&absolute, &transaction.id, index)?;
    let result = (|| {
        let restore_data = revision
            .existed
            .then(|| restore_bytes(store, revision, &forward, path))
            .transpose()?;
        let mut target = TransactionTarget {
            path: path.to_owned(),
            abs_path: absolute.to_string_lossy().into_owned(),
            restore_existed: revision.existed,
            restore_mode: revision.mode,
            restore_sha: revision.sha256.clone(),
            restore_blob: revision.blob_ref.clone(),
            restore_encoding: revision.encoding,
            forward_existed: forward.existed,
            forward_mode: forward.mode,
            forward_sha: forward.sha256.clone(),
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
            let recorded_blob = !revision.blob_ref.is_empty()
                && store
                    .blobs
                    .as_ref()
                    .is_some_and(|blobs| blobs.has(&revision.blob_ref));
            if !recorded_blob {
                target.restore_blob.clear();
                if let Some(blobs) = &store.blobs {
                    target.restore_blob = blobs.put(&data).map_err(|error| {
                        TransactionError::at(TransactionPhase::Prepare, error.into())
                    })?;
                } else {
                    target.restore_inline = Some(data.clone());
                }
            }
            target.publish_tmp = publish_tmp.to_string_lossy().into_owned();
            secure_path::write_new_file(&store.root, &publish_tmp, &data, mode)
                .map_err(|error| TransactionError::at(TransactionPhase::Prepare, error.into()))?;
        } else {
            target.action = "delete".to_owned();
        }
        if forward.existed {
            let data =
                forward
                    .content
                    .as_deref()
                    .ok_or_else(|| TransactionError::MissingPayload {
                        path: path.to_owned(),
                        reference: forward.sha256.clone(),
                    })?;
            if let Some(blobs) = &store.blobs {
                target.forward_blob = blobs.put(data).map_err(|error| {
                    TransactionError::at(TransactionPhase::Prepare, error.into())
                })?;
            } else {
                target.forward_inline = Some(data.to_vec());
            }
        }
        Ok(target)
    })();
    if result.is_err() {
        let _ = secure_path::remove(&store.root, &publish_tmp);
    }
    result
}

fn restore_bytes(
    store: &Store,
    revision: &FileRevision,
    forward: &Fingerprint,
    path: &str,
) -> Result<Vec<u8>, TransactionError> {
    if !revision.blob_ref.is_empty()
        && let Some(blobs) = &store.blobs
    {
        match load_blob(blobs, &revision.blob_ref, path) {
            Ok(data) => return Ok(data),
            Err(_) if revision.content.is_some() => {}
            Err(error) => return Err(error),
        }
    }
    if let Some(text) = revision.content.as_deref() {
        let kind = revision
            .encoding
            .or(Some(forward.encoding))
            .unwrap_or(Kind::Utf8);
        return fileencoding::encode(text, kind).map_err(|error| {
            TransactionError::at(TransactionPhase::Prepare, TransactionError::Encoding(error))
        });
    }
    if !revision.sha256.is_empty()
        && let Some(blobs) = &store.blobs
        && blobs.has(&revision.sha256)
    {
        return load_blob(blobs, &revision.sha256, path);
    }
    Err(TransactionError::MissingPayload {
        path: path.to_owned(),
        reference: if revision.blob_ref.is_empty() {
            revision.sha256.clone()
        } else {
            revision.blob_ref.clone()
        },
    })
}

pub(super) fn load_restore_payload(
    store: &Store,
    target: &TransactionTarget,
) -> Result<Vec<u8>, TransactionError> {
    if !target.restore_blob.is_empty()
        && let Some(blobs) = &store.blobs
    {
        return load_blob(blobs, &target.restore_blob, &target.path);
    }
    if let Some(inline) = &target.restore_inline {
        return Ok(inline.clone());
    }
    if !target.restore_sha.is_empty()
        && let Some(blobs) = &store.blobs
        && blobs.has(&target.restore_sha)
    {
        return load_blob(blobs, &target.restore_sha, &target.path);
    }
    Err(TransactionError::MissingPayload {
        path: target.path.clone(),
        reference: if target.restore_blob.is_empty() {
            target.restore_sha.clone()
        } else {
            target.restore_blob.clone()
        },
    })
}

fn load_blob(
    blobs: &super::blob::BlobStore,
    reference: &str,
    path: &str,
) -> Result<Vec<u8>, TransactionError> {
    match blobs.get(reference) {
        Ok(data) => Ok(data),
        Err(BlobError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Err(TransactionError::MissingPayload {
                path: path.to_owned(),
                reference: reference.to_owned(),
            })
        }
        Err(error) => Err(TransactionError::at(
            TransactionPhase::Prepare,
            error.into(),
        )),
    }
}

fn checkpoint_backup(store: &Store, from_turn: i64) -> Result<Vec<u8>, TransactionError> {
    let checkpoints: Vec<Checkpoint> = store
        .state()
        .all()
        .into_iter()
        .filter(|checkpoint| checkpoint.turn >= from_turn)
        .collect();
    gojson::marshal(&checkpoints)
        .map(|json| json.into_bytes())
        .map_err(|error| {
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
    transaction: TransactionManifest,
    cause: TransactionError,
) -> Result<TransactionManifest, TransactionError> {
    let _ = cleanup_temps(store, &transaction.targets);
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
