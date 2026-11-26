use std::collections::BTreeMap;
use std::path::Path;

use super::path_capture::normalize_rel_path;
use super::store::{Store, StoreError};
use super::types::{Checkpoint, TransactionTarget};

impl Store {
    pub fn sync_restored_ownership(&self, targets: &[TransactionTarget]) -> Result<(), StoreError> {
        self.sync_ownership(targets, |target| {
            (
                target.restore_existed,
                target.restore_sha.as_str(),
                target.restore_mode,
            )
        })
    }

    pub fn sync_forward_ownership(&self, targets: &[TransactionTarget]) -> Result<(), StoreError> {
        self.sync_ownership(targets, |target| {
            (
                target.forward_existed,
                target.forward_sha.as_str(),
                target.forward_mode,
            )
        })
    }

    fn sync_ownership(
        &self,
        targets: &[TransactionTarget],
        identity: impl Fn(&TransactionTarget) -> (bool, &str, u32),
    ) -> Result<(), StoreError> {
        let _disk = self.lock_existing_disk()?;
        let mut state = self.state();
        let checkpoints = state.all();
        let mut updates: BTreeMap<i64, Checkpoint> = BTreeMap::new();
        for target in targets {
            let path = normalize_rel_path(&self.root, Path::new(&target.path));
            let Some((checkpoint, index)) = checkpoints.iter().rev().find_map(|checkpoint| {
                checkpoint
                    .files
                    .iter()
                    .position(|file| normalize_rel_path(&self.root, Path::new(&file.path)) == path)
                    .map(|index| (checkpoint, index))
            }) else {
                continue;
            };
            let updated = updates
                .entry(checkpoint.turn)
                .or_insert_with(|| checkpoint.clone());
            let (existed, sha256, mode) = identity(target);
            let file = &mut updated.files[index];
            file.after_existed = Some(existed);
            file.after_sha256.clear();
            file.after_sha256.push_str(sha256);
            file.after_mode = mode;
        }

        for checkpoint in updates.values() {
            self.persist_checkpoint(checkpoint)?;
        }
        for checkpoint in updates.into_values() {
            if let Some(current) = state.checkpoint_mut(checkpoint.turn) {
                *current = checkpoint;
            }
        }
        Ok(())
    }
}
