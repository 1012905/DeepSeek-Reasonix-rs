use std::path::Path;

use super::disk::StoreState;
use super::ids::new_random_id;
use super::manifest::blob_name_for;
use super::types::{
    Capacity, EnqueueRequest, Error, INTENT_FOLLOWUP, INTENT_STEER, InboxItemMeta, InboxReceipt,
    InboxState, PromptEnvelope, ReadItemError, RefSnapshot, RefSummary, STATE_BLOCKED,
    STATE_QUEUED,
};
use super::{Result, encode_envelope};
use crate::sessionstore::eventlog::record::CreatedAt;

use super::preview_text;

impl super::disk::Store {
    pub fn enqueue(&self, request: EnqueueRequest) -> Result<InboxReceipt> {
        let env = complete_enqueue_envelope(request.envelope);
        let has_invocation = env.invocation.is_some() || !env.invocations.is_empty();
        if env.submit_text.is_empty()
            && env.display_text.is_empty()
            && env.raw_text.is_empty()
            && !has_invocation
        {
            return Err(Error::Empty);
        }
        let intent = if request.intent == INTENT_STEER {
            INTENT_STEER.clone()
        } else {
            INTENT_FOLLOWUP.clone()
        };
        let idem = first_non_empty(&[&request.idempotency, &env.idempotency]);
        if !idem.is_empty() {
            super::manifest::valid_idempotency_key(&idem)
                .then_some(())
                .ok_or(Error::InvalidIdempotencyKey)?;
        }
        let source = first_non_empty(&[&request.source, &env.source]);
        let (blob_bytes, checksum, byte_size) = encode_envelope(&env)?;
        let request_hash = super::idempotency_request_hash(&env)?;

        let mut state = self.lock_state();
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        self.mutable_locked(&state)?;
        if let Some(receipt) = self.idempotent_receipt_locked(&state, &idem, &request_hash)? {
            return Ok(receipt);
        }
        if idem.is_empty()
            && !request.origin.is_host()
            && let Some(receipt) =
                self.held_duplicate_locked(&mut state, &checksum, &request.origin)?
        {
            return Ok(receipt);
        }
        if byte_size > self.limits.max_item_bytes {
            return Err(Error::ItemTooLarge);
        }
        if self.limits.max_items > 0
            && i64::try_from(state.queue.manifest.items.len()).unwrap_or(i64::MAX)
                >= self.limits.max_items
        {
            return Err(Error::CapacityItems);
        }
        if state.queue.manifest.total_bytes().wrapping_add(byte_size) > self.limits.max_total_bytes
        {
            return Err(Error::CapacityBytes);
        }

        let id = new_random_id();
        let now = CreatedAt::now();
        let meta = InboxItemMeta {
            id: id.clone(),
            session_id: first_non_empty(&[
                &request.session_id,
                &agent_branch_id(&state.binding.session),
            ]),
            intent,
            state: STATE_QUEUED.clone(),
            revision: state.queue.manifest.revision.wrapping_add(1),
            blob_name: id.clone(),
            source,
            created_at: now,
            updated_at: now,
            preview: preview_text(&env.display_text, super::DEFAULT_PREVIEW_RUNES),
            byte_size,
            checksum,
            idempotency: idem.clone(),
            origin: request.origin,
            refs: ref_summaries(&env.refs),
            images: env.carries_images(),
            run_id: self.run_id.to_owned(),
            ..InboxItemMeta::default()
        };
        self.write_blob_at_locked(&state, &id, &blob_bytes)?;
        let mut next = state.queue.manifest.clone();
        next.items.push(meta);
        super::idempotency::bind_idempotency(&mut next, &idem, &id, &request_hash);
        if let Err(error) = self.commit_manifest_locked(&mut state, next) {
            let _ = self.remove_blob_at_locked(&state, &id);
            return Err(error);
        }
        let snapshot = self.snapshot_locked(&state);
        let receipt = InboxReceipt {
            item_id: id,
            disposition: super::DISPOSITION_QUEUED_FOLLOWUP.clone(),
            position: snapshot.capacity.items,
            paused: snapshot.paused,
            capacity: snapshot.capacity,
            ..InboxReceipt::default()
        };
        self.notify_locked(&state, snapshot);
        Ok(receipt)
    }

    pub fn read_item(
        &self,
        id: &str,
    ) -> std::result::Result<(InboxItemMeta, PromptEnvelope), ReadItemError> {
        let id = id.trim();
        let mut state = self.lock_state();
        let _transaction = self
            .begin_disk_transaction_locked(&mut state)
            .map_err(|source| ReadItemError {
                meta: Box::new(InboxItemMeta::default()),
                source,
            })?;
        if state.lifecycle.closed {
            return Err(ReadItemError {
                meta: Box::new(InboxItemMeta::default()),
                source: Error::Closed,
            });
        }
        let meta = state.queue.manifest.item(id).ok_or_else(|| ReadItemError {
            meta: Box::new(InboxItemMeta::default()),
            source: Error::NotFound,
        })?;
        let env = self
            .read_blob_at_locked(&state, blob_name_for(&meta), &meta.checksum)
            .map_err(|source| ReadItemError {
                meta: Box::new(meta.clone()),
                source,
            })?;
        Ok((meta, env))
    }

    pub fn update_item(&self, id: &str, env: PromptEnvelope) -> Result<InboxItemMeta> {
        self.update_item_with_idempotency(id, env, "", PromptEnvelope::default())
    }

    pub fn update_item_with_idempotency(
        &self,
        id: &str,
        env: PromptEnvelope,
        alias: &str,
        alias_envelope: PromptEnvelope,
    ) -> Result<InboxItemMeta> {
        let id = id.trim();
        let alias = alias.trim();
        if !alias.is_empty() && !super::manifest::valid_idempotency_key(alias) {
            return Err(Error::InvalidIdempotencyKey);
        }
        let env = normalize_envelope(env);
        if env.submit_text.is_empty() && env.invocation.is_none() && env.invocations.is_empty() {
            return Err(Error::Empty);
        }
        let (blob_bytes, checksum, byte_size) = encode_envelope(&env)?;
        let alias_hash = if alias.is_empty() {
            String::new()
        } else {
            super::idempotency_request_hash(&complete_enqueue_envelope(alias_envelope))?
        };

        let mut state = self.lock_state();
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        self.mutable_locked(&state)?;
        let meta = state.queue.manifest.item(id).ok_or(Error::NotFound)?;
        if !is_pending_state(&meta.state) {
            return Err(Error::InvalidState);
        }
        if self.idempotent_alias_replay_locked(&state, alias, &alias_hash, id)? {
            return Ok(meta);
        }
        if byte_size > self.limits.max_item_bytes {
            return Err(Error::ItemTooLarge);
        }
        if state
            .queue
            .manifest
            .total_bytes()
            .wrapping_add(byte_size.wrapping_sub(meta.byte_size))
            > self.limits.max_total_bytes
        {
            return Err(Error::CapacityBytes);
        }

        let old_blob = blob_name_for(&meta).to_owned();
        let new_blob = format!("{id}.{}", new_random_id());
        self.write_blob_at_locked(&state, &new_blob, &blob_bytes)?;
        let mut next = state.queue.manifest.clone();
        let index = usize::try_from(next.index_of(id)).map_err(|_| Error::NotFound)?;
        {
            let updated = &mut next.items[index];
            updated.blob_name = new_blob.clone();
            updated.byte_size = byte_size;
            updated.checksum = checksum;
            updated.preview = preview_text(&env.display_text, super::DEFAULT_PREVIEW_RUNES);
            updated.refs = ref_summaries(&env.refs);
            updated.images = env.carries_images();
            updated.updated_at = CreatedAt::now();
            updated.revision = next.revision.wrapping_add(1);
            if updated.state == STATE_BLOCKED {
                updated.state = STATE_QUEUED.clone();
                updated.block_reason.clear();
                updated.block_code = super::types::BlockCode::default();
            }
        }
        super::idempotency::bind_idempotency(&mut next, alias, id, &alias_hash);
        if let Err(error) = self.commit_manifest_locked(&mut state, next) {
            let _ = self.remove_blob_at_locked(&state, &new_blob);
            return Err(error);
        }
        let _ = self.remove_blob_at_locked(&state, &old_blob);
        let updated = state.queue.manifest.items[index].clone();
        let snapshot = self.snapshot_locked(&state);
        self.notify_locked(&state, snapshot);
        Ok(updated)
    }
}

pub(super) fn is_pending_state(state: &InboxState) -> bool {
    state == &STATE_QUEUED || state == &STATE_BLOCKED || state == &super::STATE_UNCERTAIN
}

fn normalize_envelope(mut env: PromptEnvelope) -> PromptEnvelope {
    env.display_text = env.display_text.trim().to_owned();
    env.raw_text = env.raw_text.trim().to_owned();
    env.submit_text = env.submit_text.trim().to_owned();
    env.format = env.format.trim().to_owned();
    env.idempotency = env.idempotency.trim().to_owned();
    env.source = env.source.trim().to_owned();
    env
}

fn complete_enqueue_envelope(mut env: PromptEnvelope) -> PromptEnvelope {
    env = normalize_envelope(env);
    if env.invocation.is_some() || !env.invocations.is_empty() {
        return env;
    }
    if env.submit_text.is_empty() {
        env.submit_text = first_non_empty(&[&env.raw_text, &env.display_text]);
    }
    if env.display_text.is_empty() {
        env.display_text.clone_from(&env.submit_text);
    }
    if env.raw_text.is_empty() {
        env.raw_text.clone_from(&env.submit_text);
    }
    env
}

fn ref_summaries(refs: &[RefSnapshot]) -> Vec<RefSummary> {
    refs.iter()
        .map(|reference| RefSummary {
            kind: reference.kind.clone(),
            path: first_non_empty(&[&reference.display_path, &reference.path]),
            commit: reference.commit.clone(),
            bytes: i64::try_from(reference.content.len()).unwrap_or(i64::MAX),
            preview: preview_text(
                &String::from_utf8_lossy(&reference.content),
                super::DEFAULT_PREVIEW_RUNES.min(40),
            ),
        })
        .collect()
}

fn first_non_empty(values: &[&String]) -> String {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn agent_branch_id(session_path: &Path) -> String {
    let name = session_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    name.strip_suffix(".jsonl").unwrap_or(name).to_owned()
}

pub(super) fn receipt_capacity(state: &StoreState, limits: &super::Limits) -> Capacity {
    Capacity {
        items: i64::try_from(state.queue.manifest.items.len()).unwrap_or(i64::MAX),
        max_items: limits.max_items.max(0),
        bytes: state.queue.manifest.total_bytes(),
        max_bytes: limits.max_total_bytes,
        max_item_bytes: limits.max_item_bytes,
    }
}

pub(super) fn duplicate_receipt(
    state: &StoreState,
    limits: &super::Limits,
    item_id: &str,
    position: i64,
) -> InboxReceipt {
    InboxReceipt {
        item_id: item_id.to_owned(),
        disposition: super::DISPOSITION_IDEMPOTENT_HIT.clone(),
        position,
        paused: state.queue.manifest.paused,
        capacity: receipt_capacity(state, limits),
        idempotent: true,
    }
}
