use std::collections::BTreeSet;

use super::Result;
use super::enqueue::is_pending_state;
use super::manifest::blob_name_for;
use super::types::{
    BlockCode, DISPOSITION_CANCELLED, DISPOSITION_DELETED, Disposition, Error, INTENT_FOLLOWUP,
    INTENT_STEER, InboxIntent, InboxItemMeta, InboxState, PAUSE_RECOVERED, PAUSE_USER, PauseCode,
    STATE_BLOCKED, STATE_QUEUED, STATE_RUNNING, STATE_STEER_ACCEPTED, STATE_STEER_CONSUMED,
    STATE_UNCERTAIN,
};
use crate::sessionstore::eventlog::record::CreatedAt;

impl super::disk::Store {
    pub fn delete_item(&self, id: &str) -> Result<()> {
        self.remove_item_in_state(id, is_pending_state, DISPOSITION_DELETED.clone())
    }

    pub fn cancel_accepted_steer(&self, id: &str) -> Result<()> {
        self.remove_item_in_state(
            id,
            |state| state == &STATE_STEER_ACCEPTED,
            DISPOSITION_CANCELLED.clone(),
        )
    }

    fn remove_item_in_state(
        &self,
        id: &str,
        allowed: fn(&InboxState) -> bool,
        disposition: Disposition,
    ) -> Result<()> {
        let id = id.trim();
        let mut state = self.lock_state();
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        self.mutable_locked(&state)?;
        let meta = state.queue.manifest.item(id).ok_or(Error::NotFound)?;
        if !allowed(&meta.state) {
            return Err(Error::InvalidState);
        }
        let mut next = state.queue.manifest.clone();
        let keys = next.idempotency_keys_for(id);
        next.remember_receipt(&keys, id, disposition, CreatedAt::now());
        let removed = next.remove_item(id).ok_or(Error::NotFound)?;
        self.commit_manifest_locked(&mut state, next)?;
        let _ = self.remove_blob_at_locked(&state, blob_name_for(&removed));
        let snapshot = self.snapshot_locked(&state);
        self.notify_locked(&state, snapshot);
        Ok(())
    }

    pub fn discard_pending_items(&self, ids: &[String]) -> Result<()> {
        self.discard_pending_items_owned(ids, "")
    }

    pub fn discard_pending_items_owned(&self, ids: &[String], source: &str) -> Result<()> {
        let wanted: BTreeSet<String> = ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect();
        if wanted.is_empty() {
            return Ok(());
        }
        let mut state = self.lock_state();
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        self.mutable_locked(&state)?;
        for item in &state.queue.manifest.items {
            if !wanted.contains(&item.id) || (!source.is_empty() && item.source != source) {
                continue;
            }
            if !is_pending_state(&item.state) {
                return Err(Error::InvalidState);
            }
        }

        let mut next = state.queue.manifest.clone();
        let mut removed = Vec::new();
        next.items.retain(|item| {
            let selected =
                wanted.contains(&item.id) && (source.is_empty() || item.source == source);
            if selected {
                removed.push(item.clone());
            }
            !selected
        });
        if removed.is_empty() {
            return Ok(());
        }
        let now = CreatedAt::now();
        for item in &removed {
            let keys = next.idempotency_keys_for(&item.id);
            next.remember_receipt(&keys, &item.id, Disposition::literal("discarded"), now);
            for key in keys {
                next.idempotency.remove(&key);
                next.idempotency_hashes.remove(&key);
            }
        }
        self.commit_manifest_locked(&mut state, next)?;
        for item in &removed {
            let _ = self.remove_blob_at_locked(&state, blob_name_for(item));
        }
        let snapshot = self.snapshot_locked(&state);
        self.notify_locked(&state, snapshot);
        Ok(())
    }

    pub fn move_item(&self, id: &str, to_index: i64) -> Result<()> {
        let id = id.trim();
        let mut state = self.lock_state();
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        self.mutable_locked(&state)?;
        let mut next = state.queue.manifest.clone();
        let from = usize::try_from(next.index_of(id)).map_err(|_| Error::NotFound)?;
        if !is_pending_state(&next.items[from].state) {
            return Err(Error::InvalidState);
        }
        let last = next.items.len().saturating_sub(1);
        let to_index = to_index.clamp(0, i64::try_from(last).unwrap_or(i64::MAX)) as usize;
        if from == to_index {
            return Ok(());
        }
        let item = next.items.remove(from);
        next.items.insert(to_index, item);
        self.commit_manifest_locked(&mut state, next)?;
        let snapshot = self.snapshot_locked(&state);
        self.notify_locked(&state, snapshot);
        Ok(())
    }

    pub fn pause(&self, code: PauseCode) -> Result<()> {
        self.set_paused(true, code)
    }

    pub fn resume(&self) -> Result<()> {
        self.set_paused(false, PauseCode::default())
    }

    fn set_paused(&self, paused: bool, code: PauseCode) -> Result<()> {
        let mut state = self.lock_state();
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        self.mutable_locked(&state)?;
        if state.queue.manifest.paused == paused
            && (!paused || code != PAUSE_USER || state.queue.manifest.pause_code == PAUSE_USER)
        {
            return Ok(());
        }
        let mut next = state.queue.manifest.clone();
        next.paused = paused;
        next.pause_code = code;
        if !paused {
            next.recovered = false;
            next.recovered_n = 0;
        }
        self.commit_manifest_locked(&mut state, next)?;
        let snapshot = self.snapshot_locked(&state);
        self.notify_locked(&state, snapshot);
        Ok(())
    }

    pub fn set_state(&self, id: &str, state: InboxState, block_reason: &str) -> Result<()> {
        self.set_state_coded(id, state, BlockCode::default(), block_reason)
    }

    pub fn set_state_coded(
        &self,
        id: &str,
        state: InboxState,
        code: BlockCode,
        block_reason: &str,
    ) -> Result<()> {
        let id = id.trim();
        let mut store_state = self.lock_state();
        let _transaction = self.begin_disk_transaction_locked(&mut store_state)?;
        self.mutable_locked(&store_state)?;
        let mut next = store_state.queue.manifest.clone();
        let index = usize::try_from(next.index_of(id)).map_err(|_| Error::NotFound)?;
        next.items[index].state = state;
        next.items[index].block_reason = block_reason.to_owned();
        next.items[index].block_code = code;
        next.items[index].updated_at = CreatedAt::now();
        self.commit_manifest_locked(&mut store_state, next)?;
        let snapshot = self.snapshot_locked(&store_state);
        self.notify_locked(&store_state, snapshot);
        Ok(())
    }

    pub fn claim_item(&self, id: &str) -> Result<()> {
        self.claim_item_over(id, false)
    }

    pub fn claim_item_over_pause(&self, id: &str) -> Result<()> {
        self.claim_item_over(id, true)
    }

    fn claim_item_over(&self, id: &str, over_pause: bool) -> Result<()> {
        let id = id.trim();
        let mut state = self.lock_state();
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        self.mutable_locked(&state)?;
        if state.queue.manifest.paused && !over_pause {
            return Err(Error::Paused);
        }
        let mut next = state.queue.manifest.clone();
        let index = usize::try_from(next.index_of(id)).map_err(|_| Error::NotFound)?;
        if next.items[index].state != STATE_QUEUED {
            return Err(Error::InvalidState);
        }
        next.items[index].state = STATE_RUNNING.clone();
        next.items[index].block_reason.clear();
        next.items[index].block_code = BlockCode::default();
        next.items[index].updated_at = CreatedAt::now();
        self.commit_manifest_locked(&mut state, next)?;
        let snapshot = self.snapshot_locked(&state);
        self.notify_locked(&state, snapshot);
        Ok(())
    }

    pub fn convert_intent(&self, id: &str, intent: InboxIntent) -> Result<()> {
        let intent = if intent == INTENT_STEER {
            INTENT_STEER.clone()
        } else {
            INTENT_FOLLOWUP.clone()
        };
        let id = id.trim();
        let mut state = self.lock_state();
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        self.mutable_locked(&state)?;
        let mut next = state.queue.manifest.clone();
        let index = usize::try_from(next.index_of(id)).map_err(|_| Error::NotFound)?;
        if !is_pending_state(&next.items[index].state) {
            return Err(Error::InvalidState);
        }
        next.items[index].intent = intent;
        next.items[index].updated_at = CreatedAt::now();
        self.commit_manifest_locked(&mut state, next)?;
        let snapshot = self.snapshot_locked(&state);
        self.notify_locked(&state, snapshot);
        Ok(())
    }

    pub fn ack_dequeue(&self, id: &str) -> Result<()> {
        let id = id.trim();
        let mut state = self.lock_state();
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        self.mutable_locked(&state)?;
        let mut next = state.queue.manifest.clone();
        let keys = next.idempotency_keys_for(id);
        next.remember_receipt(
            &keys,
            id,
            Disposition::literal("acknowledged"),
            CreatedAt::now(),
        );
        let removed = next.remove_item(id).ok_or(Error::NotFound)?;
        if removed.state != STATE_RUNNING
            && removed.state != STATE_STEER_ACCEPTED
            && removed.state != STATE_STEER_CONSUMED
        {
            return Err(Error::InvalidState);
        }
        self.commit_manifest_locked(&mut state, next)?;
        let _ = self.remove_blob_at_locked(&state, blob_name_for(&removed));
        let snapshot = self.snapshot_locked(&state);
        self.notify_locked(&state, snapshot);
        Ok(())
    }

    pub fn retry_item(&self, id: &str) -> Result<()> {
        let id = id.trim();
        let mut state = self.lock_state();
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        self.mutable_locked(&state)?;
        let mut next = state.queue.manifest.clone();
        let index = usize::try_from(next.index_of(id)).map_err(|_| Error::NotFound)?;
        if next.items[index].state == STATE_UNCERTAIN {
            if !next.items[index].block_code.retry_safe() {
                return Err(Error::RetryMayRepeat);
            }
        } else if next.items[index].state != STATE_BLOCKED {
            return Err(Error::InvalidState);
        }
        next.items[index].state = STATE_QUEUED.clone();
        next.items[index].block_reason.clear();
        next.items[index].block_code = BlockCode::default();
        next.items[index].updated_at = CreatedAt::now();
        self.commit_manifest_locked(&mut state, next)?;
        let snapshot = self.snapshot_locked(&state);
        self.notify_locked(&state, snapshot);
        Ok(())
    }

    pub fn next_queued(&self) -> Result<(InboxItemMeta, bool)> {
        let mut state = self.lock_state();
        if let Err(transaction) = self.begin_disk_transaction_locked(&mut state) {
            self.read_manifest_without_transaction(&mut state)
                .map_err(|read| Error::QueueRead {
                    read: Box::new(read),
                    transaction: Box::new(transaction),
                })?;
        }
        if state.queue.manifest.paused || state.queue.readonly {
            return Ok((InboxItemMeta::default(), false));
        }
        if let Some(item) = state
            .queue
            .manifest
            .items
            .iter()
            .find(|item| item.state == STATE_QUEUED && item.intent == INTENT_FOLLOWUP)
        {
            return Ok((item.clone(), true));
        }
        Ok(state
            .queue
            .manifest
            .items
            .iter()
            .find(|item| item.state == STATE_QUEUED)
            .map(|item| (item.clone(), true))
            .unwrap_or_default())
    }

    pub fn force_pause(&self, reason_recovered: bool, n: i64) -> Result<()> {
        let mut state = self.lock_state();
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        if state.queue.readonly {
            return Ok(());
        }
        if state.lifecycle.closed {
            return Err(Error::Closed);
        }
        let mut next = state.queue.manifest.clone();
        next.paused = true;
        next.pause_code = PAUSE_RECOVERED.clone();
        if reason_recovered {
            next.recovered = true;
            if n > 0 {
                next.recovered_n = n;
            }
        }
        self.commit_manifest_locked(&mut state, next)?;
        let snapshot = self.snapshot_locked(&state);
        self.notify_locked(&state, snapshot);
        Ok(())
    }

    pub fn requeue_unapplied_steer(&self, id: &str) -> Result<bool> {
        let mut state = self.lock_state();
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        self.mutable_locked(&state)?;
        let mut next = state.queue.manifest.clone();
        let index = usize::try_from(next.index_of(id)).map_err(|_| Error::NotFound)?;
        if next.items[index].state != STATE_STEER_ACCEPTED {
            return Ok(false);
        }
        next.items[index].state = STATE_QUEUED.clone();
        next.items[index].intent = INTENT_FOLLOWUP.clone();
        next.items[index].block_reason.clear();
        next.items[index].block_code = BlockCode::default();
        next.items[index].updated_at = CreatedAt::now();
        self.commit_manifest_locked(&mut state, next)?;
        let snapshot = self.snapshot_locked(&state);
        self.notify_locked(&state, snapshot);
        Ok(true)
    }
}
