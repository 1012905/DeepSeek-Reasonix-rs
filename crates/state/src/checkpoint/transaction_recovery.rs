use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::sessionstore::eventlog::record::CreatedAt;

use super::atomic_json::read_json_file;
use super::store::Store;
use super::transaction::{
    ConversationApplier, RecoveryError, RecoveryNote, RecoveryPhase, RecoveryReport,
    TransactionPhase,
};
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

    /// Runs startup recovery; conversation work is deferred when `applier` is `None`.
    pub fn recover_transactions_with_applier(
        &self,
        applier: Option<&dyn ConversationApplier>,
    ) -> RecoveryReport {
        let Some(dir) = self.dir.as_deref() else {
            return self.recover_in_memory_undo(applier);
        };
        self.recover_disk_transactions(dir, applier)
    }

    fn recover_disk_transactions(
        &self,
        dir: &Path,
        applier: Option<&dyn ConversationApplier>,
    ) -> RecoveryReport {
        let mut report = RecoveryReport::default();
        let directory = transaction_dir(dir);
        let mut entries: Vec<PathBuf> = match fs::read_dir(&directory) {
            Ok(entries) => match entries.collect::<Result<Vec<_>, _>>() {
                Ok(entries) => entries.into_iter().map(|entry| entry.path()).collect(),
                Err(source) => {
                    let error = RecoveryError::ReadDirectory {
                        path: directory.clone(),
                        source,
                    };
                    let message = error.to_string();
                    self.record_note(
                        &mut report,
                        RecoveryPhase::ReadDirectory,
                        directory,
                        message.clone(),
                    );
                    report.errors.push(error);
                    self.block_undo_recovery(pending_manifest(message));
                    return report;
                }
            },
            Err(source) => {
                if fs::symlink_metadata(&directory)
                    .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
                {
                    return report;
                }
                let error = RecoveryError::ReadDirectory {
                    path: directory.clone(),
                    source,
                };
                let message = error.to_string();
                self.record_note(
                    &mut report,
                    RecoveryPhase::ReadDirectory,
                    directory,
                    message.clone(),
                );
                report.errors.push(error);
                self.block_undo_recovery(pending_manifest(message));
                return report;
            }
        };
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
                    let error = RecoveryError::InvalidManifest { path: path.clone() };
                    let message = error.to_string();
                    self.record_note(
                        &mut report,
                        RecoveryPhase::ReadManifest,
                        path.clone(),
                        message.clone(),
                    );
                    report.errors.push(error);
                    recovering.push(pending_manifest_for_path(&path, message));
                    continue;
                }
                Err(source) => {
                    let error = RecoveryError::ReadManifest {
                        path: path.clone(),
                        source: super::secure_path::SecureError::io(
                            "stat transaction manifest",
                            &path,
                            source,
                        ),
                    };
                    let message = error.to_string();
                    self.record_note(
                        &mut report,
                        RecoveryPhase::ReadManifest,
                        path.clone(),
                        message.clone(),
                    );
                    report.errors.push(error);
                    recovering.push(pending_manifest_for_path(&path, message));
                    continue;
                }
            }
            let transaction: TransactionManifest = match read_json_file(&path) {
                Ok(transaction) => transaction,
                Err(source) => {
                    let error = RecoveryError::ReadManifest {
                        path: path.clone(),
                        source,
                    };
                    let message = error.to_string();
                    self.record_note(
                        &mut report,
                        RecoveryPhase::ReadManifest,
                        path.clone(),
                        message.clone(),
                    );
                    report.errors.push(error);
                    recovering.push(pending_manifest_for_path(&path, message));
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
            let committing = transaction.state == TransactionState::Committing;
            let transaction = self.recover_transaction(transaction, applier, &mut report);
            if committing {
                recovering.push(transaction);
            }
        }
        let offer = latest.filter(|transaction| {
            transaction.state == TransactionState::Committed
                && !undone_parents.contains(&transaction.id)
        });
        self.restore_undo_slot(offer, recovering);
        report
    }

    fn recover_in_memory_undo(&self, applier: Option<&dyn ConversationApplier>) -> RecoveryReport {
        let mut report = RecoveryReport::default();
        let Some(transaction) = self.pending_undo_transaction() else {
            return report;
        };
        let transaction = self.recover_transaction(transaction, applier, &mut report);
        self.finish_pending_undo_recovery(&transaction);
        report
    }

    fn recover_transaction(
        &self,
        mut transaction: TransactionManifest,
        applier: Option<&dyn ConversationApplier>,
        report: &mut RecoveryReport,
    ) -> TransactionManifest {
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
                        Err(source) => {
                            let error = RecoveryError::Transaction {
                                transaction_id: transaction.id.clone(),
                                source,
                            };
                            let message =
                                format!("could not persist prepared transaction abort; {error}");
                            self.record_note(
                                report,
                                RecoveryPhase::PersistAborted,
                                self.transaction_manifest_path(&transaction.id),
                                error.to_string(),
                            );
                            report.errors.push(error);
                            message
                        }
                    };
                self.record_note(
                    report,
                    RecoveryPhase::AbortPrepared,
                    self.transaction_manifest_path(&transaction.id),
                    message,
                );
            }
            TransactionState::Committing => {
                return self.recover_committing_transaction(transaction, applier, report);
            }
            TransactionState::Committed
            | TransactionState::Aborted
            | TransactionState::Undone
            | TransactionState::Invalidated => {}
        }
        transaction
    }

    fn recover_committing_transaction(
        &self,
        mut transaction: TransactionManifest,
        applier: Option<&dyn ConversationApplier>,
        report: &mut RecoveryReport,
    ) -> TransactionManifest {
        let needs_conversation = matches!(
            transaction.scope,
            RewindScope::Conversation | RewindScope::Both
        );
        if needs_conversation && applier.is_none() {
            self.record_note(
                report,
                RecoveryPhase::DeferredConversation,
                self.transaction_manifest_path(&transaction.id),
                format!("deferred conversation recovery {}", transaction.id),
            );
            return transaction;
        }
        if !matches!(transaction.kind.as_str(), "rewind" | "undo" | "file_revert") {
            let error = RecoveryError::UnsupportedKind {
                kind: transaction.kind.clone(),
            };
            self.record_note(
                report,
                RecoveryPhase::UnsupportedKind,
                self.transaction_manifest_path(&transaction.id),
                error.to_string(),
            );
            report.errors.push(error);
            return transaction;
        }
        if needs_conversation {
            let applier = applier.expect("conversation applier was checked above");
            if transaction.kind != "undo"
                && transaction.has_boundary
                && transaction.conversation_forward.is_empty()
            {
                let error = RecoveryError::MissingForwardConversation {
                    transaction_id: transaction.id.clone(),
                };
                self.retain_conversation_pending(&mut transaction, error, report);
                return transaction;
            }
            let restored = if transaction.kind == "undo" {
                transaction_compensate::reapply_rewind(&transaction, Some(applier))
            } else {
                transaction_compensate::restore_conversation(&transaction, Some(applier))
            };
            if let Err(source) = restored {
                let error = RecoveryError::Transaction {
                    transaction_id: transaction.id.clone(),
                    source,
                };
                self.retain_conversation_pending(&mut transaction, error, report);
                return transaction;
            }
        }
        self.recover_committing_files(transaction, report)
    }

    fn retain_conversation_pending(
        &self,
        transaction: &mut TransactionManifest,
        error: RecoveryError,
        report: &mut RecoveryReport,
    ) {
        let path = self.transaction_manifest_path(&transaction.id);
        let cause = match &error {
            RecoveryError::MissingForwardConversation { .. } => {
                "missing forward conversation payload".to_owned()
            }
            RecoveryError::Transaction { source, .. } => source.to_string(),
            _ => error.to_string(),
        };
        transaction.state = TransactionState::Committing;
        transaction.error = format!("crash recovery conversation compensation pending: {cause}");
        transaction.updated_at = CreatedAt::now();
        self.record_note(
            report,
            RecoveryPhase::ConversationPending,
            path.clone(),
            format!("conversation recovery {} pending: {cause}", transaction.id),
        );
        report.errors.push(error);
        if let Err(source) = persist_manifest(self, transaction, TransactionPhase::PersistAborted) {
            let message = source.to_string();
            let error = RecoveryError::Transaction {
                transaction_id: transaction.id.clone(),
                source,
            };
            transaction.error = format!("{}; {message}", transaction.error);
            self.record_note(
                report,
                RecoveryPhase::PersistAborted,
                path,
                error.to_string(),
            );
            report.errors.push(error);
        }
    }

    fn recover_committing_files(
        &self,
        mut transaction: TransactionManifest,
        report: &mut RecoveryReport,
    ) -> TransactionManifest {
        let path = self.transaction_manifest_path(&transaction.id);
        let mut stages = Vec::new();
        match transaction_compensate::compensate(self, &transaction.targets, &mut stages, None) {
            Ok(()) => {
                transaction.state = TransactionState::Aborted;
                transaction.error = "compensated after crash during commit".to_owned();
                transaction.updated_at = CreatedAt::now();
                match persist_manifest(self, &transaction, TransactionPhase::PersistAborted) {
                    Ok(()) => self.record_note(
                        report,
                        RecoveryPhase::Compensated,
                        path,
                        format!("compensated committing {}", transaction.id),
                    ),
                    Err(source) => {
                        transaction.state = TransactionState::Committing;
                        let message = format!(
                            "persist compensated transaction {}: {source}",
                            transaction.id
                        );
                        let error = RecoveryError::Transaction {
                            transaction_id: transaction.id.clone(),
                            source,
                        };
                        self.record_note(report, RecoveryPhase::PersistAborted, path, message);
                        report.errors.push(error);
                    }
                }
            }
            Err(source) => {
                transaction.state = TransactionState::Committing;
                transaction.error = format!("crash recovery compensation pending: {source}");
                transaction.updated_at = CreatedAt::now();
                let message = format!("compensate {}: {source}", transaction.id);
                let error = RecoveryError::Transaction {
                    transaction_id: transaction.id.clone(),
                    source,
                };
                self.record_note(
                    report,
                    RecoveryPhase::CompensationPending,
                    path.clone(),
                    message,
                );
                report.errors.push(error);
                if let Err(source) =
                    persist_manifest(self, &transaction, TransactionPhase::PersistAborted)
                {
                    let message = source.to_string();
                    let error = RecoveryError::Transaction {
                        transaction_id: transaction.id.clone(),
                        source,
                    };
                    transaction.error = format!("{}; {message}", transaction.error);
                    self.record_note(
                        report,
                        RecoveryPhase::PersistAborted,
                        path,
                        error.to_string(),
                    );
                    report.errors.push(error);
                }
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

    fn record_note(
        &self,
        report: &mut RecoveryReport,
        phase: RecoveryPhase,
        path: PathBuf,
        message: String,
    ) {
        report
            .notes
            .push(self.record_recovery_note(phase, path, message));
    }

    fn record_recovery_note(
        &self,
        phase: RecoveryPhase,
        path: PathBuf,
        message: String,
    ) -> RecoveryNote {
        let note = RecoveryNote {
            phase,
            path,
            message,
        };
        self.runtime
            .recovery_notes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(note.clone());
        note
    }
}

fn pending_manifest(error: String) -> TransactionManifest {
    TransactionManifest {
        state: TransactionState::Committing,
        error,
        ..TransactionManifest::default()
    }
}

fn pending_manifest_for_path(path: &Path, error: String) -> TransactionManifest {
    let mut manifest = pending_manifest(error);
    manifest.id = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    manifest
}
