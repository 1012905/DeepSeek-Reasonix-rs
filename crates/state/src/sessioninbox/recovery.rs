//! Recovery of active inbox work whose live owner is gone.

use std::collections::BTreeSet;

use super::Result;
use super::disk::Store;
use super::types::{
    BLOCK_OWNER_INACTIVE, InboxItemMeta, STATE_RUNNING, STATE_STEER_ACCEPTED, STATE_STEER_CONSUMED,
    STATE_UNCERTAIN,
};
use crate::sessionstore::eventlog::record::CreatedAt;

impl Store {
    /// Recovers active items whose exact ids are absent from the owned set.
    pub fn recover_orphaned_in_flight(&self, owned_ids: &[String]) -> Result<i64> {
        let owned: BTreeSet<&str> = owned_ids
            .iter()
            .map(String::as_str)
            .filter(|id| !id.is_empty())
            .collect();
        self.recover_orphaned_in_flight_owned_by(Some(&|id| owned.contains(id)))
    }

    /// The callback runs under the store mutex and must not call Store methods.
    pub fn recover_orphaned_in_flight_owned_by(
        &self,
        owned_by: Option<&dyn Fn(&str) -> bool>,
    ) -> Result<i64> {
        let mut state = self.lock_state();
        if !needs_recovery(&state.queue.manifest.items, owned_by) {
            return Ok(0);
        }
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        self.mutable_locked(&state)?;

        let mut next = state.queue.manifest.clone();
        let recovered = recover_items(&mut next.items, owned_by);
        if recovered == 0 {
            return Ok(0);
        }
        next.paused = true;
        next.recovered = true;
        next.recovered_n = i64::try_from(next.items.len())
            .unwrap_or(i64::MAX)
            .min(next.recovered_n.wrapping_add(recovered));
        self.commit_manifest_locked(&mut state, next)?;
        let snapshot = self.snapshot_locked(&state);
        self.notify_locked(&state, snapshot);
        Ok(recovered)
    }
}

fn needs_recovery(items: &[InboxItemMeta], owned_by: Option<&dyn Fn(&str) -> bool>) -> bool {
    let mut needed = false;
    for item in items {
        let owned = owned_by.is_some_and(|is_owned| is_owned(&item.id));
        if !owned && is_active(&item.state) {
            needed = true;
        }
    }
    needed
}

fn recover_items(items: &mut [InboxItemMeta], owned_by: Option<&dyn Fn(&str) -> bool>) -> i64 {
    let now = CreatedAt::now();
    let mut recovered = 0i64;
    for item in items {
        let owned = owned_by.is_some_and(|is_owned| is_owned(&item.id));
        if owned || !is_active(&item.state) {
            continue;
        }
        item.state = STATE_UNCERTAIN.clone();
        item.block_reason = "in-flight owner is no longer active".to_owned();
        item.block_code = BLOCK_OWNER_INACTIVE.clone();
        item.updated_at = now;
        recovered = recovered.wrapping_add(1);
    }
    recovered
}

fn is_active(state: &super::InboxState) -> bool {
    state == &STATE_RUNNING || state == &STATE_STEER_ACCEPTED || state == &STATE_STEER_CONSUMED
}
