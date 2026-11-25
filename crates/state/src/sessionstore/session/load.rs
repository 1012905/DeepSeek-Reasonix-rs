//! The session load chain (Go: LoadSession and the decode ladder in
//! session_events.go, session_checkpoint_agreement.go, and the
//! read-model guards of session_image_blobs.go).
//!
//! Contract: docs/contracts/session-format.md 7.12 (SESS-023).
//!
//! A load is: prepare the path, replay the authoritative log (falling back
//! to the JSONL checkpoint), normalize in memory, and anchor the persistence
//! baseline against the branch-meta ledger. Normalization is persisted
//! lazily by the next save, so a read-only session is never rewritten.

use std::borrow::Cow;
use std::fs;
use std::path::Path;
use std::sync::RwLock;

use reasonix_contract::provider::{Message, normalize_session_messages};

use super::canonical::with_session_save_path_lock;
use super::digest::digest_and_size_session_messages;
use super::save::LoadError;
use super::storage_eq::{messages_have_prefix, messages_have_prefix_with_compatible_system};
use super::{Session, SessionState};
use crate::sessionstore::branch::load_branch_meta_retry;
use crate::sessionstore::dag::load_session_dag_messages;
use crate::sessionstore::eventlog::{
    SessionEventReplay, holds_any_image, is_data_url, probe_session_event_log,
    replay_session_event_log, session_image_key, write_session_image_blob,
};
use crate::sessionstore::preview::migrate_legacy_provider_content;
use crate::sessionstore::v4_import::prepare_session_path as prepare_v4_import;
use crate::store::naming::{
    session_blobs_dir, session_cleanup_pending, session_event_log, session_v4_removed_marker,
};

/// Loads a session under the save-path lock (Go: LoadSession).
///
/// The session file lock is not taken here — Go's LoadSession does not take it
/// either; only the in-process save-path lock serializes a load against a save.
pub fn load_session(path: &str) -> Result<Session, LoadError> {
    prepare_session_path(Path::new(path))?;
    with_session_save_path_lock(path, || load_session_unlocked(Path::new(path)))
}

/// Makes a listed path openable (Go session_v4_import.go
/// `PrepareSessionPath`).
///
/// A removed v4 import stays removed even while its native artifacts remain;
/// otherwise an unchanged import is completed or a source continued by 1.x is
/// refreshed before the native load chain opens it.
pub fn prepare_session_path(session_path: &Path) -> Result<(), LoadError> {
    prepare_v4_import(session_path).map_err(|err| LoadError::V4Import(Box::new(err)))
}

/// Loads a session without taking the file locks (Go:
/// loadSessionUnlocked): replay the event log, normalize, and anchor the
/// persisted baseline against the branch-meta ledger.
pub fn load_session_unlocked(session_path: &Path) -> Result<Session, LoadError> {
    let (msgs, _from_events, damaged) = load_session_messages(session_path)?;
    let normalized = normalize_session_messages(&msgs);
    let normalize_dirty = matches!(normalized, Cow::Owned(_));
    let migrated = migrate_legacy_provider_content(&normalized);
    let dirty = normalize_dirty || matches!(migrated, Cow::Owned(_));
    let messages = migrated.into_owned();
    let session = Session {
        state: RwLock::new(SessionState {
            messages,
            event_log_damaged: damaged,
            normalized_dirty: dirty,
            raw_messages: if dirty { msgs } else { Vec::new() },
            ..SessionState::default()
        }),
    };
    // Bind the digest first: the read guard must drop before the
    // mark_persisted* calls below take the write lock (Rust 2024 keeps
    // if-let scrutinee temporaries alive through the arm).
    let anchored = digest_and_size_session_messages(&session.read().messages).ok();
    if let Some((digest, _content_bytes)) = anchored {
        let path = session_path.to_string_lossy();
        let version = session.transcript_version();
        let rewrite_version = session.rewrite_version();
        match load_branch_meta_retry(session_path) {
            Err(_) => {
                session.mark_persisted_revision_unknown(&path, &digest, version, rewrite_version);
            }
            Ok(meta) => {
                let mut revision = 0;
                if let Some(meta) = &meta {
                    revision = meta.revision;
                }
                let ledger_digest = match &meta {
                    Some(meta) => meta.content_digest.as_str(),
                    None => "",
                };
                if session.load_agrees_with_ledger(meta.is_some(), ledger_digest, digest) {
                    session.mark_persisted(&path, &digest, version, revision, rewrite_version);
                } else {
                    session.mark_persisted_from_load(
                        &path,
                        &digest,
                        version,
                        revision,
                        rewrite_version,
                    );
                }
            }
        }
    }
    Ok(session)
}

/// Loads the stored messages (Go: loadSessionMessages): the event log
/// when this build owns it, the JSONL checkpoint otherwise. Returns the
/// messages, whether they came from the event log, and whether the log
/// replay was damaged.
pub fn load_session_messages(session_path: &Path) -> Result<(Vec<Message>, bool, bool), LoadError> {
    let probe = probe_session_event_log(session_path)?;
    if probe.future_schema {
        return Err(LoadError::UnownedEventLog);
    }
    if probe.dag {
        let (msgs, damaged) = load_session_dag_messages(session_path)?;
        return Ok((msgs, false, damaged));
    }
    if probe.native && probe.size > 0 {
        let replay = replay_session_event_log(&session_event_log(session_path))?;
        if replay.records > 0 {
            return resolve_replayed_messages(session_path, replay);
        }
        // Records but none replayed: fall back to the JSONL defensively.
        return load_session_messages_from_jsonl(session_path).map(|msgs| (msgs, false, true));
    }
    load_session_messages_from_jsonl(session_path).map(|msgs| (msgs, false, false))
}

/// Reconciles a damaged replay against the checkpoint (Go:
/// resolveReplayedMessages): a checkpoint that extends the replay heals
/// the log's tail; otherwise the decoded replay wins.
fn resolve_replayed_messages(
    session_path: &Path,
    replay: SessionEventReplay,
) -> Result<(Vec<Message>, bool, bool), LoadError> {
    if replay.damaged
        && let Some(healed) = checkpoint_extending_replay(session_path, &replay.msgs)
    {
        return Ok((healed, false, true));
    }
    Ok((replay.msgs, true, replay.damaged))
}

/// Heals a damaged replay from a longer checkpoint (Go:
/// checkpointExtendingReplay): the checkpoint must strictly extend the
/// replayed prefix.
fn checkpoint_extending_replay(session_path: &Path, replayed: &[Message]) -> Option<Vec<Message>> {
    let msgs = match load_session_messages_from_jsonl(session_path) {
        Ok(msgs) => msgs,
        Err(_) => load_session_messages_from_jsonl_keeping_decoded_tail(session_path),
    };
    if msgs.len() <= replayed.len() {
        return None;
    }
    if !messages_have_prefix(&msgs, replayed)
        && !messages_have_prefix_with_compatible_system(&msgs, replayed)
    {
        return None;
    }
    Some(msgs)
}

/// Strict JSONL load (Go: loadSessionMessagesFromJSONL): any decode
/// error names the file.
pub(crate) fn load_session_messages_from_jsonl(
    session_path: &Path,
) -> Result<Vec<Message>, LoadError> {
    let data = fs::read(session_path)?;
    let mut msgs = Vec::new();
    for item in serde_json::Deserializer::from_slice(&data).into_iter::<Message>() {
        match item {
            Ok(msg) => msgs.push(msg),
            Err(source) => {
                return Err(LoadError::Decode {
                    path: session_path.to_string_lossy().into_owned(),
                    source,
                });
            }
        }
    }
    Ok(msgs)
}

/// Lenient JSONL load keeping the decoded prefix (Go:
/// loadSessionMessagesFromJSONLKeepingDecodedTail): a torn final line is
/// dropped; any other decode error gives up entirely.
pub(crate) fn load_session_messages_from_jsonl_keeping_decoded_tail(
    session_path: &Path,
) -> Vec<Message> {
    let Ok(data) = fs::read(session_path) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&data);
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.iter().rposition(|line| !line.trim().is_empty());
    let mut msgs = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Message>(line) {
            Ok(msg) => msgs.push(msg),
            Err(_) => {
                if Some(i) != last {
                    return Vec::new();
                }
                return msgs;
            }
        }
    }
    msgs
}

/// Loads the baseline session for a CAS check (Go:
/// loadSnapshotBaseline), refusing saves that would drop the only copy
/// of a stored image and restoring data-URL image blobs the caller
/// lost.
pub fn load_snapshot_baseline(session_path: &Path, next: &[Message]) -> Result<Session, LoadError> {
    let current = load_session_unlocked(session_path)?;
    let stored = current.snapshot();
    for (i, m) in next.iter().enumerate() {
        if !m.unavailable_images.is_empty()
            && i < stored.len()
            && holds_any_image(&stored[i], &m.unavailable_images)
        {
            return Err(LoadError::ImagesLost);
        }
    }
    let dir = session_blobs_dir(session_path);
    let mut restored = false;
    for (i, m) in stored.iter().enumerate() {
        if m.unavailable_images.is_empty() || i >= next.len() {
            continue;
        }
        for url in &next[i].images {
            if !is_data_url(url)
                || !m
                    .unavailable_images
                    .iter()
                    .any(|lost| lost.reference == session_image_key(url))
            {
                continue;
            }
            write_session_image_blob(&dir, url)?;
            restored = true;
        }
    }
    if !restored {
        return Ok(current);
    }
    load_session_unlocked(session_path)
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

/// Go save.go `IsCleanupPending`: a delayed-cleanup marker hides a
/// logically removed session from resume/list surfaces until delayed
/// physical cleanup has finished.
pub fn is_cleanup_pending(session_path: &str) -> bool {
    let marker = session_cleanup_pending(Path::new(session_path));
    if marker.as_os_str().is_empty() {
        return false;
    }
    marker.exists()
}

/// Go session_v4_removal.go `isV4SessionRemoved`: a deleted 1.x import keeps
/// a `.v4-removed` tombstone so the v4 discovery pass cannot re-import it.
/// The marker path itself is the whole rule; only a `v4-<id>.jsonl` name in a
/// sessions folder has one.
fn is_v4_session_removed(session_path: &str) -> bool {
    let marker = session_v4_removed_marker(Path::new(session_path));
    if marker.as_os_str().is_empty() {
        return false;
    }
    marker.exists()
}

/// Go save.go `IsVisibleSession`: whether a persisted session should
/// appear on normal user/agent-facing list, restore, and retrieval
/// surfaces.
pub fn is_visible_session(session_path: &str) -> bool {
    !session_path.trim().is_empty()
        && !is_cleanup_pending(session_path)
        && !is_v4_session_removed(session_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::eventlog::test_support::tempdir;
    fn user_message(content: &str) -> Message {
        Message {
            role: reasonix_contract::provider::Role::USER,
            content: content.to_string(),
            ..Message::default()
        }
    }

    fn session_of(messages: Vec<Message>) -> Session {
        let session = Session::new("");
        session.replace(messages);
        session
    }

    #[test]
    fn a_v4_tombstone_hides_only_a_v4_import() {
        let dir = tempdir();
        let sessions = dir.path().join("sessions");
        fs::create_dir_all(&sessions).expect("sessions dir");
        let id = "0123456789abcdef0123456789abcdef";
        let imported = sessions.join(format!("v4-{id}.jsonl"));
        let plain = sessions.join("20260101-000000.000000000-m.jsonl");
        fs::write(
            &imported, b"
",
        )
        .expect("write import");
        fs::write(
            &plain, b"
",
        )
        .expect("write plain");

        assert!(is_visible_session(&imported.to_string_lossy()));
        assert!(is_visible_session(&plain.to_string_lossy()));

        // Only a v4-imported name in a sessions folder owns a tombstone path.
        fs::write(crate::store::session_v4_removed_marker(&imported), b"{}")
            .expect("write tombstone");
        assert!(!is_visible_session(&imported.to_string_lossy()));
        assert!(is_visible_session(&plain.to_string_lossy()));

        // A v4-looking name outside a sessions folder has no tombstone rule.
        let elsewhere = dir.path().join("other").join(format!("v4-{id}.jsonl"));
        fs::create_dir_all(elsewhere.parent().expect("parent")).expect("dir");
        fs::write(
            &elsewhere, b"
",
        )
        .expect("write");
        assert!(
            crate::store::session_v4_removed_marker(&elsewhere)
                .as_os_str()
                .is_empty()
        );
        assert!(is_visible_session(&elsewhere.to_string_lossy()));
    }

    #[test]
    fn the_v4_id_rule_admits_only_1x_ids() {
        assert!(crate::store::is_v4_session_id(
            "0123456789abcdef0123456789abcdef"
        ));
        assert!(crate::store::is_v4_session_id("0123456789abcdef"));
        assert!(crate::store::is_v4_session_id(&format!(
            "desktop-{}",
            "0123456789abcdef012345678"
        )));
        assert!(crate::store::is_v4_session_id(&format!(
            "desktop-manual-{}",
            "0123456789abcdef012345678"
        )));
        // Uppercase, too short, too long, and non-hex all fail.
        assert!(!crate::store::is_v4_session_id(
            "0123456789ABCDEF0123456789abcdef"
        ));
        assert!(!crate::store::is_v4_session_id("0123456789abcde"));
        assert!(!crate::store::is_v4_session_id(&"0".repeat(65)));
        assert!(!crate::store::is_v4_session_id("zzzzzzzzzzzzzzzz"));
    }

    fn write_dag(session: &Path, records: &[String], torn: bool) {
        fs::write(session, b"").expect("write checkpoint");
        let mut body = records.join("\n");
        body.push('\n');
        if torn {
            body.push_str("{\"schema_version\":2,\"type\":");
        }
        fs::write(session_event_log(session), body).expect("write DAG");
    }

    fn dag_message(id: &str, parent: &str, head: &str, role: &str, content: &str) -> String {
        format!(
            "{{\"schema_version\":2,\"at\":\"2026-09-25T13:00:00Z\",\"type\":\"message\",\"id\":\"{id}\",\"parent\":\"{parent}\",\"head\":\"{head}\",\"msgs\":[{{\"role\":\"{role}\",\"content\":{}}}]}}",
            serde_json::to_string(content).expect("content JSON")
        )
    }

    #[test]
    fn load_session_reads_the_selected_foreign_dag_and_marks_a_torn_tail() {
        let dir = tempdir();
        let path = dir.path().join("foreign.jsonl");
        write_dag(
            &path,
            &[
                dag_message("u", "", "main", "user", "hello from 1.x"),
                dag_message("a", "u", "main", "assistant", "answer from 1.x"),
            ],
            true,
        );

        let session = load_session(&path.to_string_lossy()).expect("load foreign DAG");
        let messages = session.snapshot();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "hello from 1.x");
        assert_eq!(messages[1].content, "answer from 1.x");
        assert!(session.read().event_log_damaged);
    }

    #[test]
    fn load_session_imports_a_v4_source_through_prepare_path() {
        use crate::sessionstore::sessionv4::test_support::{End, Fixture, ID, message};

        let mut fixture = Fixture::new(ID);
        fixture.batch(
            vec![
                message("u", "user", "imported question"),
                message("a", "assistant", "imported answer"),
            ],
            End::Ended,
        );
        let path = crate::sessionstore::v4_import::v4_import_path(&fixture.sessions(), ID);

        let session = load_session(&path.to_string_lossy()).expect("prepare and load v4 import");
        let messages = session.snapshot();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "imported question");
        assert_eq!(messages[1].content, "imported answer");
        assert!(session_event_log(&path).exists());
        let meta = crate::sessionstore::branch::load_branch_meta(&path)
            .expect("branch meta")
            .expect("imported branch meta");
        assert_eq!(
            meta.imported_from.expect("import source").line,
            "1.x sessions-v4"
        );
    }

    #[test]
    fn cleanup_pending_hides_a_session() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        session_of(vec![user_message("hello world")])
            .save(&path_str)
            .expect("save");

        assert!(is_visible_session(&path_str));
        assert!(!is_visible_session(""));
        assert!(!is_visible_session("   "));
        assert!(!is_cleanup_pending(""));

        let marker = crate::store::naming::session_cleanup_pending(&path);
        std::fs::write(&marker, "{}").expect("write cleanup marker");
        assert!(is_cleanup_pending(&path_str));
        assert!(!is_visible_session(&path_str));

        std::fs::remove_file(&marker).expect("remove cleanup marker");
        assert!(!is_cleanup_pending(&path_str));
        assert!(is_visible_session(&path_str));
    }
}
