use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::sessionstore::eventlog::record::CreatedAt;

use super::atomic_json::read_json_file;
use super::store::Store;
use super::transaction::{RecoveryNote, RecoveryPhase, TransactionPhase};
use super::transaction_compensate;
use super::transaction_prepare::{cleanup_temps, persist_manifest};
use super::types::{RewindScope, TransactionManifest, TransactionState, transaction_dir};

impl Store {
    pub fn recovery_notes(&self) -> Vec<RecoveryNote> {
        self.runtime
            .recovery_notes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(super) fn recover_file_transactions_on_startup(&self) {
        let Some(dir) = self.dir.as_deref() else {
            return;
        };
        let directory = transaction_dir(dir);
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => match entries.collect::<Result<Vec<_>, _>>() {
                Ok(entries) => entries,
                Err(error) => {
                    self.record_recovery_note(
                        RecoveryPhase::ReadDirectory,
                        directory.clone(),
                        error.to_string(),
                    );
                    self.restore_undo_slot(None, vec![pending_manifest(error.to_string())]);
                    return;
                }
            },
            Err(error) => {
                if fs::symlink_metadata(&directory)
                    .is_err_and(|source| source.kind() == io::ErrorKind::NotFound)
                {
                    return;
                }
                self.record_recovery_note(
                    RecoveryPhase::ReadDirectory,
                    directory,
                    error.to_string(),
                );
                self.restore_undo_slot(None, vec![pending_manifest(error.to_string())]);
                return;
            }
        };
        let mut entries: Vec<PathBuf> = entries.into_iter().map(|entry| entry.path()).collect();
        entries.sort();
        let mut latest: Option<TransactionManifest> = None;
        let mut undone_parents = HashSet::new();
        let mut recovering = Vec::new();
        for path in entries {
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => continue,
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    recovering.push(self.record_pending_manifest(
                        path,
                        "transaction manifest is not a regular file".to_owned(),
                    ));
                    continue;
                }
                Err(error) => {
                    recovering.push(self.record_pending_manifest(path, error.to_string()));
                    continue;
                }
            }
            let transaction: TransactionManifest = match read_json_file(&path) {
                Ok(transaction) => transaction,
                Err(error) => {
                    recovering.push(self.record_pending_manifest(path, error.to_string()));
                    continue;
                }
            };
            if transaction.state == TransactionState::Committed
                && transaction.kind == "undo"
                && !transaction.parent_transaction.is_empty()
            {
                undone_parents.insert(transaction.parent_transaction.clone());
            }
            if transaction.kind != "undo"
                && matches!(
                    transaction.state,
                    TransactionState::Committed
                        | TransactionState::Invalidated
                        | TransactionState::Undone
                )
                && latest
                    .as_ref()
                    .is_none_or(|current| current.created_at.get() < transaction.created_at.get())
            {
                latest = Some(transaction.clone());
            }
            if transaction.state == TransactionState::Committing {
                recovering.push(self.recover_transaction(transaction));
            } else {
                self.recover_transaction(transaction);
            }
        }
        let offer = latest.filter(|transaction| {
            transaction.state == TransactionState::Committed
                && !undone_parents.contains(&transaction.id)
        });
        self.restore_undo_slot(offer, recovering);
    }

    fn recover_transaction(&self, mut transaction: TransactionManifest) -> TransactionManifest {
        match transaction.state {
            TransactionState::Prepared => {
                let cleanup = cleanup_temps(self, &transaction.targets);
                transaction.state = TransactionState::Aborted;
                transaction.error = "abandoned prepared transaction on recovery".to_owned();
                transaction.updated_at = CreatedAt::now();
                let message =
                    match persist_manifest(self, &transaction, TransactionPhase::PersistAborted) {
                        Ok(()) => cleanup
                            .map(|_| "aborted prepared transaction".to_owned())
                            .unwrap_or_else(|error| {
                                format!("aborted prepared transaction; {error}")
                            }),
                        Err(error) => {
                            self.record_recovery_note(
                                RecoveryPhase::PersistAborted,
                                self.transaction_manifest_path(&transaction.id),
                                error.to_string(),
                            );
                            format!("could not persist prepared transaction abort; {error}")
                        }
                    };
                self.record_recovery_note(
                    RecoveryPhase::AbortPrepared,
                    self.transaction_manifest_path(&transaction.id),
                    message,
                );
            }
            TransactionState::Committing => {
                if matches!(
                    transaction.scope,
                    RewindScope::Conversation | RewindScope::Both
                ) {
                    self.record_recovery_note(
                        RecoveryPhase::DeferredConversation,
                        self.transaction_manifest_path(&transaction.id),
                        "conversation-aware recovery requires SESS-051".to_owned(),
                    );
                    return transaction;
                }
                if transaction.kind != "rewind" && transaction.kind != "undo" {
                    self.record_recovery_note(
                        RecoveryPhase::UnsupportedKind,
                        self.transaction_manifest_path(&transaction.id),
                        format!("unsupported transaction kind {}", transaction.kind),
                    );
                    return transaction;
                }
                return self.recover_committing_file_transaction(transaction);
            }
            TransactionState::Committed
            | TransactionState::Aborted
            | TransactionState::Undone
            | TransactionState::Invalidated => {}
        }
        transaction
    }

    fn recover_committing_file_transaction(
        &self,
        mut transaction: TransactionManifest,
    ) -> TransactionManifest {
        let path = self.transaction_manifest_path(&transaction.id);
        let mut stages = Vec::new();
        match transaction_compensate::compensate(self, &transaction.targets, &mut stages, None) {
            Ok(()) => {
                transaction.state = TransactionState::Aborted;
                if transaction.error.is_empty() {
                    transaction.error = "compensated committing transaction on recovery".to_owned();
                }
                transaction.updated_at = CreatedAt::now();
                match persist_manifest(self, &transaction, TransactionPhase::PersistAborted) {
                    Ok(()) => self.record_recovery_note(
                        RecoveryPhase::Compensated,
                        path,
                        "compensated file transaction".to_owned(),
                    ),
                    Err(error) => {
                        transaction.state = TransactionState::Committing;
                        self.record_recovery_note(
                            RecoveryPhase::PersistAborted,
                            path,
                            error.to_string(),
                        );
                    }
                }
            }
            Err(error) => {
                transaction.state = TransactionState::Committing;
                transaction.error = join_attribution(&transaction.error, &error);
                transaction.updated_at = CreatedAt::now();
                let message =
                    match persist_manifest(self, &transaction, TransactionPhase::PersistAborted) {
                        Ok(()) => transaction.error.clone(),
                        Err(persist) => join_attribution(&transaction.error, &persist),
                    };
                self.record_recovery_note(RecoveryPhase::CompensationPending, path, message);
            }
        }
        transaction
    }

    pub(super) fn transaction_manifest_path(&self, id: &str) -> PathBuf {
        self.dir
            .as_deref()
            .map(|dir| transaction_dir(dir).join(format!("{id}.json")))
            .unwrap_or_default()
    }

    fn record_pending_manifest(&self, path: PathBuf, error: String) -> TransactionManifest {
        self.record_recovery_note(RecoveryPhase::ReadManifest, path.clone(), error.clone());
        pending_manifest_for_path(&path, error)
    }

    fn record_recovery_note(&self, phase: RecoveryPhase, path: PathBuf, message: String) {
        self.runtime
            .recovery_notes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(RecoveryNote {
                phase,
                path,
                message,
            });
    }
}

fn pending_manifest(error: String) -> TransactionManifest {
    TransactionManifest {
        state: TransactionState::Committing,
        error,
        ..TransactionManifest::default()
    }
}

fn pending_manifest_for_path(path: &std::path::Path, error: String) -> TransactionManifest {
    let mut manifest = pending_manifest(error);
    manifest.id = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    manifest
}

fn join_attribution(previous: &str, error: &super::transaction::TransactionError) -> String {
    if previous.is_empty() {
        error.to_string()
    } else {
        format!("{previous}; {error}")
    }
}
