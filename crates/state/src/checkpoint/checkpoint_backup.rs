use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use super::store::{Store, StoreError, turn_path};
use super::types::Checkpoint;

impl Store {
    pub(super) fn restore_checkpoint_backup(&self, backup: &[u8]) -> Result<(), StoreError> {
        let path = self
            .dir
            .as_deref()
            .unwrap_or_else(|| Path::new("."))
            .join("checkpoint-backup.json");
        let restored: Vec<Checkpoint> =
            serde_json::from_slice(backup).map_err(|source| StoreError::Json {
                path: path.clone(),
                source,
            })?;
        let _disk = self.lock_existing_disk()?;
        let mut state = self.state();
        let mut by_turn: BTreeMap<i64, Checkpoint> = state
            .all()
            .into_iter()
            .map(|checkpoint| (checkpoint.turn, checkpoint))
            .collect();
        let Some(dir) = self.dir.as_deref() else {
            for checkpoint in restored {
                by_turn.insert(checkpoint.turn, checkpoint);
            }
            state.done = by_turn.into_values().collect();
            state.current = None;
            state.seen.clear();
            return Ok(());
        };

        for checkpoint in restored {
            let turn = checkpoint.turn;
            self.persist_checkpoint(&checkpoint)?;
            let counterpart = turn_path(dir, turn, !checkpoint.expired_file_payload);
            match fs::remove_file(&counterpart) {
                Ok(()) => {}
                Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(StoreError::Io {
                        op: "remove stale checkpoint counterpart",
                        path: counterpart,
                        source,
                    });
                }
            }
            by_turn.insert(turn, checkpoint);
        }
        state.done = by_turn.into_values().collect();
        state.current = None;
        state.seen.clear();
        Ok(())
    }
}
