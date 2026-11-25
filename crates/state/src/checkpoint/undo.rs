use std::fmt;
use std::sync::{Arc, Mutex};

use crate::sessionstore::eventlog::record::CreatedAt;

use super::atomic_json::write_json_atomic;
use super::store::{Store, StoreError};
use super::types::{TransactionManifest, TransactionState};

#[derive(Clone)]
pub(super) struct UndoCallback(Arc<dyn Fn() + Send + Sync>);

impl fmt::Debug for UndoCallback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UndoCallback")
    }
}

#[derive(Debug, Default)]
pub(super) struct UndoRuntime {
    operation: Mutex<()>,
    current: Mutex<Option<TransactionManifest>>,
    on_change: Mutex<Option<UndoCallback>>,
}

impl Store {
    pub fn on_undo_change<F>(&self, callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        *self
            .runtime
            .undo
            .on_change
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(UndoCallback(Arc::new(callback)));
    }

    #[cfg(test)]
    pub(crate) fn current_undo_offer(&self) -> Option<TransactionManifest> {
        self.runtime
            .undo
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn invalidate_undo(&self) -> Result<(), StoreError> {
        let previous = {
            let _operation = self
                .runtime
                .undo
                .operation
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let mut current = self
                .runtime
                .undo
                .current
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(offer) = current.as_ref() else {
                return Ok(());
            };
            let mut invalidated = offer.clone();
            invalidated.state = TransactionState::Invalidated;
            invalidated.updated_at = CreatedAt::now();
            if self.dir.is_some() {
                let path = self.transaction_manifest_path(&invalidated.id);
                write_json_atomic(&path, &invalidated).map_err(|source| {
                    StoreError::UndoInvalidation {
                        id: invalidated.id.clone(),
                        source,
                    }
                })?;
            }
            let previous = invalidated.id;
            *current = None;
            previous
        };
        self.notify_undo_change();
        debug_assert!(!previous.is_empty());
        Ok(())
    }

    pub(super) fn publish_undo_offer(&self, transaction: &TransactionManifest) {
        debug_assert_eq!(transaction.state, TransactionState::Committed);
        let changed = {
            let _operation = self
                .runtime
                .undo
                .operation
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let mut current = self
                .runtime
                .undo
                .current
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let changed =
                current.as_ref().map(|offer| offer.id.as_str()) != Some(transaction.id.as_str());
            if changed {
                *current = Some(transaction.clone());
            }
            changed
        };
        if changed {
            self.notify_undo_change();
        }
    }

    pub(super) fn restore_undo_offer(&self, transaction: Option<TransactionManifest>) {
        let _operation = self
            .runtime
            .undo
            .operation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *self
            .runtime
            .undo
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner()) =
            transaction.filter(|transaction| transaction.state == TransactionState::Committed);
    }

    fn notify_undo_change(&self) {
        let callback = self
            .runtime
            .undo
            .on_change
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(callback) = callback {
            callback.0();
        }
    }
}
