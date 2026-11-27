//! Manifest refresh and durable publication.

use std::path::Path;

use super::Result;
use super::disk::StoreState;
use super::disk_fs::{ensure_private_dir, is_not_found, read_regular_file_in, write_atomic_file};
use super::manifest::{Manifest, decode_manifest, empty_manifest, validate_manifest};
use super::types::{
    BLOCK_OWNER_INACTIVE, Capacity, Error, InboxSnapshot, PAUSE_RECOVERED, SCHEMA_VERSION,
    STATE_BLOCKED, STATE_QUEUED, STATE_RUNNING, STATE_STEER_ACCEPTED, STATE_STEER_CONSUMED,
    STATE_UNCERTAIN,
};
use crate::sessionstore::eventlog::record::CreatedAt;

pub(super) const MANIFEST_NAME: &str = "manifest.json";
pub(super) const MAX_MANIFEST_BYTES: u64 = 8 << 20;

impl super::disk::Store {
    pub(super) fn load_or_init_locked(&self, state: &mut StoreState) -> Result<()> {
        let inbox = ensure_private_dir(&state.binding.dir)?;
        let path = state.binding.dir.join(MANIFEST_NAME);
        let data =
            match read_regular_file_in(&inbox, Path::new(MANIFEST_NAME), &path, MAX_MANIFEST_BYTES)
            {
                Ok(data) => data,
                Err(err) if is_not_found(&err) => {
                    state.queue.manifest = empty_manifest(self.run_id);
                    state.queue.readonly = false;
                    state.queue.unrecovered = false;
                    return Ok(());
                }
                Err(err) => return Err(err),
            };
        let mut manifest = match decode_manifest(&data) {
            Ok(manifest) => manifest,
            Err(_) => return self.recover_corrupt_manifest_locked(state),
        };
        state.queue.unrecovered = false;
        if manifest.schema_version > SCHEMA_VERSION {
            state.queue.manifest = manifest;
            state.queue.readonly = true;
            state.queue.manifest.paused = true;
            return Ok(());
        }
        let migrated = manifest.schema_version < SCHEMA_VERSION;
        if migrated {
            self.backfill_idempotency_hashes_at(state, &mut manifest)?;
            manifest.schema_version = SCHEMA_VERSION;
        }
        let recovered = recover_foreign_run(&mut manifest, self.run_id);
        manifest.run_id = self.run_id.to_owned();
        state.queue.manifest = manifest;
        state.queue.readonly = false;
        if recovered > 0 || migrated {
            return self.commit_current_manifest_locked(state);
        }
        self.gc_orphans_locked(state);
        Ok(())
    }

    pub(super) fn read_manifest_without_transaction(&self, state: &mut StoreState) -> Result<()> {
        let inbox = match super::disk_fs::validate_private_dir(&state.binding.dir) {
            Ok(inbox) => inbox,
            Err(err) if is_not_found(&err) => {
                state.queue.manifest = empty_manifest(self.run_id);
                state.queue.readonly = false;
                return Ok(());
            }
            Err(err) => return Err(err),
        };
        let path = state.binding.dir.join(MANIFEST_NAME);
        let data =
            match read_regular_file_in(&inbox, Path::new(MANIFEST_NAME), &path, MAX_MANIFEST_BYTES)
            {
                Ok(data) => data,
                Err(err) if is_not_found(&err) => {
                    state.queue.manifest = empty_manifest(self.run_id);
                    state.queue.readonly = false;
                    return Ok(());
                }
                Err(err) => return Err(err),
            };
        let mut manifest = decode_manifest(&data)?;
        if !manifest.run_id.is_empty() && manifest.run_id != self.run_id {
            return Err(Error::ForeignRun);
        }
        state.queue.readonly = manifest.schema_version > SCHEMA_VERSION;
        if state.queue.readonly {
            manifest.paused = true;
        }
        state.queue.manifest = manifest;
        Ok(())
    }

    pub(super) fn commit_manifest_locked(
        &self,
        state: &mut StoreState,
        mut next: Manifest,
    ) -> Result<()> {
        Self::publish_manifest(
            &state.binding.dir,
            self.run_id,
            state.queue.unrecovered,
            &mut next,
        )?;
        state.queue.manifest = next;
        Ok(())
    }

    pub(super) fn commit_current_manifest_locked(&self, state: &mut StoreState) -> Result<()> {
        Self::publish_manifest(
            &state.binding.dir,
            self.run_id,
            state.queue.unrecovered,
            &mut state.queue.manifest,
        )
    }

    fn publish_manifest(
        dir: &Path,
        run_id: &str,
        unrecovered: bool,
        next: &mut Manifest,
    ) -> Result<()> {
        if unrecovered {
            return Err(Error::Unrecovered);
        }
        validate_manifest(next, false)?;
        let inbox = ensure_private_dir(dir)?;
        next.schema_version = SCHEMA_VERSION;
        next.run_id = run_id.to_owned();
        next.revision = next.revision.wrapping_add(1);
        next.updated_at = CreatedAt::now();
        let pretty = serde_json::to_string_pretty(next)?;
        let mut data = reasonix_contract::gojson::escape_html(&pretty).into_bytes();
        data.push(b'\n');
        let path = dir.join(MANIFEST_NAME);
        reasonix_core::fileutil::crash("inbox-manifest-write", &path.to_string_lossy());
        write_atomic_file(&inbox, Path::new(MANIFEST_NAME), &path, &data)?;
        reasonix_core::fileutil::crash("inbox-manifest-commit", &path.to_string_lossy());
        Ok(())
    }

    pub(super) fn snapshot_locked(&self, state: &StoreState) -> InboxSnapshot {
        let manifest = &state.queue.manifest;
        let item_count = i64::try_from(manifest.items.len()).unwrap_or(i64::MAX);
        let items = if manifest.items.is_empty() {
            None
        } else {
            Some(manifest.items.clone())
        };
        InboxSnapshot {
            schema_version: manifest.schema_version,
            revision: manifest.revision,
            paused: manifest.paused || state.queue.unrecovered,
            pause_code: manifest.pause_code.clone(),
            recovered: manifest.recovered,
            recovered_n: manifest.recovered_n,
            unrecovered: state.queue.unrecovered,
            readonly: state.queue.readonly,
            run_id: manifest.run_id.clone(),
            capacity: Capacity {
                items: item_count,
                max_items: self.limits.max_items.max(0),
                bytes: manifest.total_bytes(),
                max_bytes: self.limits.max_total_bytes,
                max_item_bytes: self.limits.max_item_bytes,
            },
            items,
        }
    }

    pub(super) fn mutable_locked(&self, state: &StoreState) -> Result<()> {
        if state.lifecycle.closed {
            return Err(Error::Closed);
        }
        if state.queue.readonly {
            return Err(Error::SchemaReadonly);
        }
        Ok(())
    }
}

fn recover_foreign_run(manifest: &mut Manifest, run_id: &str) -> i64 {
    if manifest.run_id.is_empty() || manifest.run_id == run_id {
        return 0;
    }
    let now = CreatedAt::now();
    let mut recovered = 0i64;
    for item in &mut manifest.items {
        if item.state == STATE_RUNNING
            || item.state == STATE_STEER_ACCEPTED
            || item.state == STATE_STEER_CONSUMED
        {
            item.state = STATE_UNCERTAIN.clone();
            item.block_reason = "in-flight owner is no longer active".to_owned();
            item.block_code = BLOCK_OWNER_INACTIVE.clone();
            item.updated_at = now;
            recovered = recovered.wrapping_add(1);
        } else if item.state == STATE_QUEUED
            || item.state == STATE_BLOCKED
            || item.state == STATE_UNCERTAIN
        {
            recovered = recovered.wrapping_add(1);
        }
    }
    if recovered > 0 || !manifest.items.is_empty() {
        manifest.paused = true;
        manifest.pause_code = PAUSE_RECOVERED.clone();
        manifest.recovered = true;
        manifest.recovered_n = recovered;
    }
    recovered
}
