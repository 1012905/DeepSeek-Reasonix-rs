use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use super::store::{Store, StoreError, turn_path};
use super::types::Checkpoint;

pub(super) fn restore(store: &Store, backup: &[u8]) -> Result<(), StoreError> {
    let backup_path = store
        .dir
        .as_deref()
        .unwrap_or_else(|| Path::new(""))
        .join("checkpoint-backup.json");
    let checkpoints: Vec<Checkpoint> =
        serde_json::from_slice(backup).map_err(|source| StoreError::Json {
            path: backup_path,
            source,
        })?;
    let _disk = store.lock_existing_disk()?;
    let mut state = store.state();
    let current_turn = state.current.as_ref().map(|checkpoint| checkpoint.turn);
    let mut by_turn: BTreeMap<i64, Checkpoint> = state
        .done
        .iter()
        .cloned()
        .map(|checkpoint| (checkpoint.turn, checkpoint))
        .collect();
    if let Some(current) = &state.current {
        by_turn.insert(current.turn, current.clone());
    }
    for checkpoint in checkpoints {
        store.persist_checkpoint(&checkpoint)?;
        if let Some(dir) = &store.dir {
            let counterpart = turn_path(dir, checkpoint.turn, !checkpoint.expired_file_payload);
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
        }
        by_turn.insert(checkpoint.turn, checkpoint);
    }
    let mut merged: Vec<Checkpoint> = by_turn.into_values().collect();
    state.current = current_turn.and_then(|turn| {
        merged
            .iter()
            .position(|checkpoint| checkpoint.turn == turn)
            .map(|index| merged.remove(index))
    });
    state.done = merged;
    Ok(())
}
