use std::path::Path;

use thiserror::Error;

use super::path_capture::lexical_clean;
use super::store::{Store, StoreError};
use super::types::{Checkpoint, Coverage, SCHEMA_V2};

#[derive(Debug, Error)]
pub enum ConversationForkError {
    #[error("conversation checkpoint destination unavailable")]
    Unavailable,
    #[error("conversation checkpoint destination is the source")]
    SourceIsDestination,
    #[error("copy conversation checkpoint {turn}: {source}")]
    Persist {
        turn: i64,
        #[source]
        source: StoreError,
    },
}

impl Store {
    pub fn copy_conversation_prefix_to(
        &self,
        dir: &Path,
        session_id: &str,
        message_count: i64,
    ) -> Result<(), ConversationForkError> {
        let Some(source_dir) = self.dir.as_deref() else {
            return Err(ConversationForkError::Unavailable);
        };
        if dir.as_os_str().is_empty() {
            return Err(ConversationForkError::Unavailable);
        }
        if lexical_clean(dir) == lexical_clean(source_dir) {
            return Err(ConversationForkError::SourceIsDestination);
        }
        let checkpoints = {
            let state = self.state();
            state
                .all()
                .into_iter()
                .filter(|checkpoint| {
                    checkpoint.msg_index >= 0 && checkpoint.msg_index < message_count
                })
                .map(|checkpoint| Checkpoint {
                    schema_version: SCHEMA_V2,
                    turn: checkpoint.turn,
                    time: checkpoint.time,
                    prompt: checkpoint.prompt,
                    msg_index: checkpoint.msg_index,
                    session_id: session_id.to_owned(),
                    files: Vec::new(),
                    coverage: Coverage::None,
                    fork_copied: true,
                    ..Checkpoint::default()
                })
                .collect::<Vec<_>>()
        };
        let destination = Store::persistence_only(dir.to_path_buf());
        for checkpoint in checkpoints {
            destination
                .persist_checkpoint(&checkpoint)
                .map_err(|source| ConversationForkError::Persist {
                    turn: checkpoint.turn,
                    source,
                })?;
        }
        Ok(())
    }
}
