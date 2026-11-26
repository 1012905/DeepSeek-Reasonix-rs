use super::Result;
use super::disk::StoreState;
use super::enqueue::{duplicate_receipt, receipt_capacity};
use super::manifest::{IDEMPOTENCY_RECEIPT_TTL, Manifest};
use super::types::{
    BLOCK_STEER_UNAPPLIED, Disposition, Error, INTENT_FOLLOWUP, InboxReceipt, PromptOrigin,
    STATE_QUEUED, STATE_UNCERTAIN,
};
use crate::sessionstore::eventlog::record::CreatedAt;

impl super::disk::Store {
    pub(super) fn idempotent_receipt_locked(
        &self,
        state: &StoreState,
        key: &str,
        request_hash: &str,
    ) -> Result<Option<InboxReceipt>> {
        if key.is_empty() {
            return Ok(None);
        }
        if let Some(item_id) = state.queue.manifest.idempotency.get(key)
            && let Some(item) = state.queue.manifest.item(item_id)
        {
            if let Some(previous) = state.queue.manifest.idempotency_hashes.get(key)
                && !previous.is_empty()
                && previous != request_hash
            {
                return Err(Error::IdempotencyConflict);
            }
            return Ok(Some(duplicate_receipt(
                state,
                &self.limits,
                &item.id,
                state.queue.manifest.index_of(&item.id).wrapping_add(1),
            )));
        }
        let Some(receipt) = state.queue.manifest.receipts.get(key) else {
            return Ok(None);
        };
        if receipt_expired(receipt.completed_at) {
            return Ok(None);
        }
        if receipt.request_hash != request_hash {
            return Err(Error::IdempotencyConflict);
        }
        let mut result = duplicate_receipt(state, &self.limits, &receipt.item_id, 0);
        result.capacity = receipt_capacity(state, &self.limits);
        Ok(Some(result))
    }

    pub(super) fn idempotent_alias_replay_locked(
        &self,
        state: &StoreState,
        key: &str,
        request_hash: &str,
        item_id: &str,
    ) -> Result<bool> {
        if key.is_empty() {
            return Ok(false);
        }
        if let Some(existing_id) = state.queue.manifest.idempotency.get(key) {
            if let Some(existing_hash) = state.queue.manifest.idempotency_hashes.get(key)
                && !existing_hash.is_empty()
                && existing_hash != request_hash
            {
                return Err(Error::IdempotencyConflict);
            }
            if existing_id != item_id {
                return Err(Error::IdempotencyConflict);
            }
            return Ok(true);
        }
        let Some(receipt) = state.queue.manifest.receipts.get(key) else {
            return Ok(false);
        };
        if receipt_expired(receipt.completed_at) {
            return Ok(false);
        }
        if receipt.request_hash != request_hash || receipt.item_id != item_id {
            return Err(Error::IdempotencyConflict);
        }
        Ok(true)
    }

    pub(super) fn held_duplicate_locked(
        &self,
        state: &mut StoreState,
        checksum: &str,
        origin: &PromptOrigin,
    ) -> Result<Option<InboxReceipt>> {
        if checksum.is_empty() {
            return Ok(None);
        }
        for index in 0..state.queue.manifest.items.len() {
            let item = state.queue.manifest.items[index].clone();
            if item.checksum != checksum || &item.origin != origin {
                continue;
            }
            if item.state == STATE_UNCERTAIN && item.block_code == BLOCK_STEER_UNAPPLIED {
                let mut next = state.queue.manifest.clone();
                let updated = &mut next.items[index];
                updated.state = STATE_QUEUED.clone();
                updated.intent = INTENT_FOLLOWUP.clone();
                updated.block_reason.clear();
                updated.block_code = super::types::BlockCode::default();
                updated.updated_at = CreatedAt::now();
                self.commit_manifest_locked(state, next)?;
                let snapshot = self.snapshot_locked(state);
                self.notify_locked(state, snapshot);
            } else if item.state != STATE_QUEUED {
                continue;
            }
            return Ok(Some(duplicate_receipt(
                state,
                &self.limits,
                &item.id,
                (index as i64).wrapping_add(1),
            )));
        }
        Ok(None)
    }

    pub fn receipt_disposition(&self, key: &str) -> Result<(Disposition, bool)> {
        let mut state = self.lock_state();
        if state.lifecycle.closed {
            return Err(Error::Closed);
        }
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        Ok(state
            .queue
            .manifest
            .receipts
            .get(key)
            .map(|receipt| (receipt.disposition.clone(), true))
            .unwrap_or_default())
    }
}

pub(super) fn bind_idempotency(
    manifest: &mut Manifest,
    key: &str,
    item_id: &str,
    request_hash: &str,
) {
    if key.is_empty() {
        return;
    }
    manifest
        .idempotency
        .insert(key.to_owned(), item_id.to_owned());
    manifest
        .idempotency_hashes
        .insert(key.to_owned(), request_hash.to_owned());
}

fn receipt_expired(completed_at: CreatedAt) -> bool {
    CreatedAt::now()
        .get()
        .signed_duration_since(completed_at.get())
        > IDEMPOTENCY_RECEIPT_TTL
}
