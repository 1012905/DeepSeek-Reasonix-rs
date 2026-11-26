//! Schema migration, foreign-owner recovery, quarantine, and salvage.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cap_std::fs::Dir;

use super::disk::StoreState;
use super::disk_fs::{
    ensure_private_dir, io_error, is_not_found, open_private_child_dir, quarantine_name,
    read_regular_file_in, validate_private_child_dir, validate_private_dir,
};
use super::disk_manifest::MANIFEST_NAME;
use super::idempotency_request_hash;
use super::manifest::{Manifest, blob_name_for, empty_manifest, valid_blob_stem};
use super::types::{
    BLOCK_MANIFEST_SALVAGED, Error, INTENT_FOLLOWUP, InboxItemMeta, PAUSE_RECOVERED,
    PromptEnvelope, STATE_UNCERTAIN,
};
use super::{Result, preview_text, sha256_hex};
use crate::sessionstore::eventlog::record::CreatedAt;

pub(super) const BLOBS_NAME: &str = "blobs";
pub(super) const QUARANTINE_NAME: &str = "quarantine";
pub(super) const BLOB_SUFFIX: &str = ".json";

impl super::disk::Store {
    pub(super) fn backfill_idempotency_hashes_at(
        &self,
        state: &StoreState,
        manifest: &mut Manifest,
    ) -> Result<()> {
        let keys: Vec<String> = manifest
            .idempotency
            .keys()
            .filter(|key| {
                manifest
                    .idempotency_hashes
                    .get(*key)
                    .is_none_or(String::is_empty)
            })
            .cloned()
            .collect();
        if keys.is_empty() {
            return Ok(());
        }
        for key in keys {
            let Some(item_id) = manifest.idempotency.get(&key) else {
                continue;
            };
            let meta = manifest.item(item_id).ok_or(Error::NotFound)?;
            let env = self.read_blob_at_locked(state, blob_name_for(&meta), &meta.checksum)?;
            let hash = idempotency_request_hash(&env)?;
            manifest.idempotency_hashes.insert(key, hash);
        }
        Ok(())
    }

    pub(super) fn recover_corrupt_manifest_locked(&self, state: &mut StoreState) -> Result<()> {
        match self.salvage_orphan_blobs_locked(state) {
            Err(_) => {
                state.queue.manifest = empty_manifest(self.run_id);
                state.queue.manifest.paused = true;
                state.queue.readonly = false;
                state.queue.unrecovered = true;
                Ok(())
            }
            Ok(items) => {
                // Go intentionally ignores quarantine failure and publishes the recovered manifest.
                let _ = self.quarantine_file_locked(state, MANIFEST_NAME, Some("manifest-corrupt"));
                let mut next = empty_manifest(self.run_id);
                next.paused = true;
                next.pause_code = PAUSE_RECOVERED.clone();
                next.recovered = true;
                next.recovered_n = i64::try_from(items.len()).unwrap_or(i64::MAX);
                next.items = items;
                state.queue.manifest = next;
                state.queue.unrecovered = false;
                self.commit_current_manifest_locked(state)
            }
        }
    }

    fn salvage_orphan_blobs_locked(&self, state: &StoreState) -> Result<Vec<InboxItemMeta>> {
        let inbox = validate_private_dir(&state.binding.dir)?;
        let blobs = match validate_private_child_dir(
            &inbox,
            BLOBS_NAME.as_ref(),
            &state.binding.dir.join(BLOBS_NAME),
        ) {
            Ok(blobs) => blobs,
            Err(err) if is_not_found(&err) => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let entries = sorted_entries(&blobs, &state.binding.dir.join(BLOBS_NAME))?;
        let now = CreatedAt::now();
        let mut items = Vec::new();
        for (name, file_type) in entries {
            if file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let Some(file_name) = name.to_str() else {
                return Err(Error::InvalidItemPath);
            };
            let Some(stem) = file_name.strip_suffix(BLOB_SUFFIX) else {
                continue;
            };
            if !valid_blob_stem(stem) {
                continue;
            }
            let path = state.binding.dir.join(BLOBS_NAME).join(&name);
            let data = read_regular_file_in(&blobs, &name, &path, self.max_item_bytes())?;
            let Ok(Some(env)) = serde_json::from_slice::<Option<PromptEnvelope>>(&data) else {
                continue;
            };
            let preview = first_non_empty(&[&env.display_text, &env.submit_text, &env.raw_text]);
            let preview = if preview.is_empty() {
                "(salvaged body)".to_owned()
            } else {
                preview_text(&preview, super::types::DEFAULT_PREVIEW_RUNES)
            };
            items.push(InboxItemMeta {
                id: stem.to_owned(),
                intent: INTENT_FOLLOWUP.clone(),
                state: STATE_UNCERTAIN.clone(),
                blob_name: stem.to_owned(),
                created_at: now,
                updated_at: now,
                preview,
                byte_size: i64::try_from(data.len()).unwrap_or(i64::MAX),
                checksum: sha256_hex(&data),
                run_id: self.run_id.to_owned(),
                block_reason: "salvaged after corrupt manifest".to_owned(),
                block_code: BLOCK_MANIFEST_SALVAGED.clone(),
                ..InboxItemMeta::default()
            });
        }
        Ok(items)
    }

    pub(super) fn quarantine_file_locked(
        &self,
        state: &StoreState,
        source_name: &str,
        tag: Option<&str>,
    ) -> Result<()> {
        let inbox = ensure_private_dir(&state.binding.dir)?;
        let quarantine = open_private_child_dir(
            &inbox,
            QUARANTINE_NAME.as_ref(),
            &state.binding.dir.join(QUARANTINE_NAME),
        )?;
        let destination = quarantine_name(Path::new(source_name).as_os_str(), tag);
        inbox
            .rename(Path::new(source_name), &quarantine, Path::new(&destination))
            .map_err(|source| {
                io_error(
                    "quarantine inbox file",
                    &state.binding.dir.join(source_name),
                    source,
                )
            })
    }

    pub(super) fn gc_orphans_locked(&self, state: &StoreState) {
        let Ok(inbox) = validate_private_dir(&state.binding.dir) else {
            return;
        };
        let blobs_path = state.binding.dir.join(BLOBS_NAME);
        let Ok(blobs) = validate_private_child_dir(&inbox, BLOBS_NAME.as_ref(), &blobs_path) else {
            return;
        };
        let Ok(entries) = sorted_entries(&blobs, &blobs_path) else {
            return;
        };
        let live: BTreeSet<&str> = state
            .queue
            .manifest
            .items
            .iter()
            .map(blob_name_for)
            .collect();
        for (name, file_type) in entries {
            if file_type.is_dir() {
                continue;
            }
            let Some(file_name) = name.to_str() else {
                let _ = self.quarantine_blob_locked(state, &name);
                continue;
            };
            if !file_name.ends_with(BLOB_SUFFIX) {
                let _ = self.quarantine_blob_locked(state, &name);
                continue;
            }
            let stem = file_name.strip_suffix(BLOB_SUFFIX).unwrap_or_default();
            if !live.contains(stem) {
                let _ = self.quarantine_blob_locked(state, &name);
            }
        }
    }

    fn quarantine_blob_locked(&self, state: &StoreState, name: &Path) -> Result<()> {
        let inbox = validate_private_dir(&state.binding.dir)?;
        let blobs_path = state.binding.dir.join(BLOBS_NAME);
        let blobs = validate_private_child_dir(&inbox, BLOBS_NAME.as_ref(), &blobs_path)?;
        let quarantine = open_private_child_dir(
            &inbox,
            QUARANTINE_NAME.as_ref(),
            &state.binding.dir.join(QUARANTINE_NAME),
        )?;
        let Some(base) = name.file_name() else {
            return Err(Error::InvalidItemPath);
        };
        let destination = quarantine_name(base, None);
        blobs
            .rename(name, &quarantine, Path::new(&destination))
            .map_err(|source| io_error("quarantine inbox blob", &blobs_path.join(name), source))
    }

    fn max_item_bytes(&self) -> u64 {
        u64::try_from(self.limits.max_item_bytes).unwrap_or(u64::MAX)
    }
}

fn sorted_entries(dir: &Dir, path: &Path) -> Result<Vec<(PathBuf, cap_std::fs::FileType)>> {
    let mut entries = Vec::new();
    for entry in dir
        .entries()
        .map_err(|source| io_error("read inbox directory", path, source))?
    {
        let entry = entry.map_err(|source| io_error("read inbox entry", path, source))?;
        let file_type = entry
            .file_type()
            .map_err(|source| io_error("inspect inbox entry", path, source))?;
        entries.push((PathBuf::from(entry.file_name()), file_type));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(entries)
}

fn first_non_empty(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
        .to_owned()
}
