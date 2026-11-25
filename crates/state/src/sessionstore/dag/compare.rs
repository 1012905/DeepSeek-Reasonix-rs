use std::path::Path;

use reasonix_contract::provider::{Message, Role, normalize_session_messages};

use crate::sessionstore::digest_session_messages;
use crate::sessionstore::preview::migrate_legacy_provider_content;

use super::load_session_dag_messages;

pub fn dag_transcript_matches(session_path: &Path, snapshot: &[Message]) -> bool {
    let Ok((messages, _)) = load_session_dag_messages(session_path) else {
        return false;
    };
    let normalized = normalize_session_messages(&messages);
    let migrated = migrate_legacy_provider_content(&normalized);
    let on_disk = digest_session_messages(without_leading_system(&migrated));
    let held = digest_session_messages(without_leading_system(snapshot));
    matches!((on_disk, held), (Ok(on_disk), Ok(held)) if on_disk == held)
}

fn without_leading_system(messages: &[Message]) -> &[Message] {
    if messages
        .first()
        .is_some_and(|message| message.role == Role::SYSTEM)
    {
        &messages[1..]
    } else {
        messages
    }
}
