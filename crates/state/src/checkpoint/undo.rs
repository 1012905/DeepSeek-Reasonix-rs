use std::fmt;
use std::ops::{Deref, DerefMut};
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
struct UndoOffers {
    current: Option<TransactionManifest>,
    pending: Option<TransactionManifest>,
}

#[derive(Debug, Default)]
pub(super) struct UndoRuntime {
    operation: Mutex<()>,
    offers: Mutex<UndoOffers>,
    on_change: Mutex<Option<UndoCallback>>,
}

pub(super) struct UndoTransaction<'store, 'transaction> {
    store: &'store Store,
    transaction: &'transaction mut TransactionManifest,
    previous_offer: Option<String>,
}

impl UndoOffers {
    fn visible_offer(&self) -> Option<&TransactionManifest> {
        if self.pending.is_none()
            && self
                .current
                .as_ref()
                .is_some_and(|transaction| transaction.state == TransactionState::Committed)
        {
            self.current.as_ref()
        } else {
            None
        }
    }
}

impl Deref for UndoTransaction<'_, '_> {
    type Target = TransactionManifest;

    fn deref(&self) -> &Self::Target {
        self.transaction
    }
}

impl DerefMut for UndoTransaction<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.transaction
    }
}

impl Drop for UndoTransaction<'_, '_> {
    fn drop(&mut self) {
        let changed = {
            let _operation = self
                .store
                .runtime
                .undo
                .operation
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let mut offers = self
                .store
                .runtime
                .undo
                .offers
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if offers
                .pending
                .as_ref()
                .is_none_or(|pending| pending.id != self.transaction.id)
            {
                return;
            }
            if matches!(
                self.transaction.state,
                TransactionState::Committed | TransactionState::Aborted
            ) {
                offers.pending = None;
            } else {
                offers.pending = Some(self.transaction.clone());
            }
            self.previous_offer.as_deref() != offers.visible_offer().map(|offer| offer.id.as_str())
        };
        if changed {
            self.store.notify_undo_change();
        }
    }
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
            .offers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .visible_offer()
            .cloned()
    }

    pub(super) fn require_undo_recovery_complete(&self) -> Result<(), StoreError> {
        if self
            .runtime
            .undo
            .offers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending
            .is_some()
        {
            Err(StoreError::UndoRecoveryPending)
        } else {
            Ok(())
        }
    }

    pub(super) fn track_undo_transaction<'store, 'transaction>(
        &'store self,
        transaction: &'transaction mut TransactionManifest,
    ) -> Result<UndoTransaction<'store, 'transaction>, StoreError> {
        let _operation = self
            .runtime
            .undo
            .operation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut offers = self
            .runtime
            .undo
            .offers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if offers.pending.is_some() {
            return Err(StoreError::UndoRecoveryPending);
        }
        let previous_offer = offers.visible_offer().map(|offer| offer.id.clone());
        let mut pending = transaction.clone();
        pending.state = TransactionState::Committing;
        offers.pending = Some(pending);
        Ok(UndoTransaction {
            store: self,
            transaction,
            previous_offer,
        })
    }

    pub fn invalidate_undo(&self) -> Result<(), StoreError> {
        let previous = {
            let _operation = self
                .runtime
                .undo
                .operation
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let mut offers = self
                .runtime
                .undo
                .offers
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if offers.pending.is_some() {
                return Err(StoreError::UndoRecoveryPending);
            }
            let Some(offer) = offers.current.as_ref() else {
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
            offers.current = None;
            previous
        };
        self.notify_undo_change();
        debug_assert!(!previous.is_empty());
        Ok(())
    }

    pub(super) fn publish_undo_offer(&self, transaction: &TransactionManifest) {
        debug_assert_eq!(transaction.state, TransactionState::Committed);
        let _operation = self
            .runtime
            .undo
            .operation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut offers = self
            .runtime
            .undo
            .offers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        offers.current = Some(transaction.clone());
    }

    pub(super) fn restore_undo_slot(
        &self,
        latest: Option<TransactionManifest>,
        recovering: Vec<TransactionManifest>,
    ) {
        let changed = {
            let _operation = self
                .runtime
                .undo
                .operation
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let mut offers = self
                .runtime
                .undo
                .offers
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let restored = UndoOffers {
                current: latest
                    .filter(|transaction| transaction.state == TransactionState::Committed),
                pending: recovering
                    .into_iter()
                    .find(|transaction| transaction.state == TransactionState::Committing),
            };
            let changed = offers.visible_offer().map(|offer| offer.id.as_str())
                != restored.visible_offer().map(|offer| offer.id.as_str());
            *offers = restored;
            changed
        };
        if changed {
            self.notify_undo_change();
        }
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
