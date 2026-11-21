//! Transcript checkpoint writes and the save-path guards around them (Go:
//! writeSessionMessages, keepSupersededTranscript, healEmptyCheckpointFromWAL,
//! probeRefusesSave, and the listing projection).
//!
//! Contract: docs/contracts/session-format.md 7.3-7.5 (SESS-017).
//!
//! The checkpoint is the compatibility JSONL read model: written whole, via a
//! temp file plus rename, after the authoritative event log has already
//! changed. Nothing writes it before the log.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use reasonix_contract::gojson;
use reasonix_contract::provider::Message;
use reasonix_core::fileutil::{create_temp_file, replace_file};

use super::load::load_session_messages;
use super::save::parent_dir;
use super::save::{SaveError, WriteMessagesError};
use crate::sessionstore::branch::{BRANCH_META_COUNTS_VERSION, update_branch_meta};
use crate::sessionstore::eventlog::record::now_utc;
use crate::sessionstore::eventlog::{
    SESSION_EVENT_IMAGE_BLOBS_SCHEMA_VERSION, SessionEventLogProbe, probe_session_event_log,
    refuse_degraded_checkpoint_rewrite,
};
use crate::sessionstore::sessionpreview::session_preview_from_messages;
use crate::store::naming::{session_event_log, session_sidecar_files, session_superseded};

/// Whether any durable artifact of the session exists (Go:
/// sessionArtifactExists): the transcript, a sidecar, or an event log
/// this build recognizes. A probe failure counts as existing.
pub fn session_artifact_exists(session_path: &Path) -> bool {
    if session_path.symlink_metadata().is_ok() {
        return true;
    }
    for sidecar in session_sidecar_files(session_path) {
        if sidecar == session_event_log(session_path) {
            match probe_session_event_log(session_path) {
                // An unreadable log is treated as present.
                Err(_) => return true,
                Ok(probe) => {
                    if !probe.native {
                        continue;
                    }
                }
            }
        }
        if sidecar.symlink_metadata().is_ok() {
            return true;
        }
    }
    false
}

/// Writes the transcript checkpoint (Go: writeSessionMessages): a temp
/// file in the session directory, one HTML-escaped JSON message per
/// line, fsync, then the atomic replace. The image-payload guard runs
/// first: a degraded checkpoint holding the only copy of an image is
/// never rewritten.
pub fn write_session_messages(
    session_path: &Path,
    msgs: &[Message],
) -> Result<(), WriteMessagesError> {
    refuse_degraded_checkpoint_rewrite(session_path, msgs)?;
    let (mut file, tmp_path) = create_temp_file(&parent_dir(session_path), ".session.", ".tmp")
        .map_err(WriteMessagesError::CreateTmp)?;
    for msg in msgs {
        let line = gojson::marshal(msg).map_err(WriteMessagesError::Encode)?;
        if let Err(err) = file
            .write_all(line.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
        {
            let _ = fs::remove_file(&tmp_path);
            return Err(WriteMessagesError::EncodeIo(err));
        }
    }
    if let Err(err) = file.sync_all() {
        let _ = fs::remove_file(&tmp_path);
        return Err(WriteMessagesError::Io(err));
    }
    drop(file);
    if let Err(err) = replace_file(&tmp_path, session_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(WriteMessagesError::Io(err));
    }
    Ok(())
}

/// Keeps the overwritten transcript for the lease audit trail (Go:
/// keepSupersededTranscript). Warn-only: failures never block a save.
pub(crate) fn keep_superseded_transcript(path: &str, superseding: bool) {
    if !superseding || path.is_empty() {
        return;
    }
    let current = match fs::read(path) {
        Ok(current) => current,
        Err(_) => return,
    };
    if current.is_empty() {
        return;
    }
    let dest = session_superseded(Path::new(path));
    if dest.as_os_str().is_empty() {
        return;
    }
    let stamp = serde_json::to_string(&now_utc())
        .unwrap_or_default()
        .trim_matches('"')
        .to_string();
    let header = format!("// superseded at {stamp} by the runtime holding this session's lease\n");
    #[cfg(unix)]
    let mut options = {
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut options = OpenOptions::new();
        options.mode(0o600);
        options
    };
    #[cfg(not(unix))]
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    let mut file = match options.open(&dest) {
        Ok(file) => file,
        Err(err) => {
            tracing::warn!(
                path = path,
                error = %err,
                "session: could not keep the superseded transcript"
            );
            return;
        }
    };
    if let Err(err) = file
        .write_all(header.as_bytes())
        .and_then(|()| file.write_all(&current))
    {
        tracing::warn!(
            path = path,
            error = %err,
            "session: could not keep the superseded transcript"
        );
    }
}

/// Rebuilds a missing or empty transcript checkpoint from the event log
/// (Go: healEmptyCheckpointFromWAL). Load failures are swallowed: there
/// is nothing to heal from then.
pub(crate) fn heal_empty_checkpoint_from_wal(session_path: &Path) -> Result<(), SaveError> {
    let need_heal = match fs::metadata(session_path) {
        Ok(info) => info.len() == 0,
        Err(err) => err.kind() == io::ErrorKind::NotFound,
    };
    if !need_heal {
        return Ok(());
    }
    let probe = probe_session_event_log(session_path)?;
    if !probe.native || probe.size <= 0 || probe.future_schema {
        return Ok(());
    }
    let Ok((msgs, from_events, damaged)) = load_session_messages(session_path) else {
        return Ok(());
    };
    if !from_events || damaged || msgs.is_empty() {
        return Ok(());
    }
    write_session_messages(session_path, &msgs).map_err(SaveError::HealFromWal)
}

/// Refuses saves onto logs this build cannot own (Go: probeRefusesSave):
/// future schemas and 1.x DAG logs.
pub(crate) fn probe_refuses_save(
    path: &str,
    probe: &SessionEventLogProbe,
) -> Result<(), SaveError> {
    if probe.future_schema {
        return Err(SaveError::FutureSchema {
            path: path.to_string(),
            version: probe.schema_version,
            max: SESSION_EVENT_IMAGE_BLOBS_SCHEMA_VERSION,
        });
    }
    if probe.dag {
        return Err(SaveError::DagForeign {
            path: path.to_string(),
        });
    }
    Ok(())
}

/// Refreshes the branch-meta listing projection (Go:
/// persistSessionListingProjection). Warn-only: a deferred listing never
/// fails the save.
pub(crate) fn persist_session_listing_projection(session_path: &Path, msgs: &[Message]) {
    let (preview, turns) = session_preview_from_messages(msgs);
    let result = update_branch_meta(session_path, false, |meta| {
        meta.preview = preview;
        meta.turns = turns;
        meta.schema_version = BRANCH_META_COUNTS_VERSION;
        Ok(())
    });
    if let Err(err) = result {
        tracing::warn!(
            path = %session_path.display(),
            error = %err,
            "session: listing metadata update deferred"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::eventlog::test_support::tempdir;
    use crate::sessionstore::session::baseline::{FileStampOnDisk, file_stamp, stamp_baseline};
    use crate::sessionstore::session::{
        Session, load_session_messages_from_jsonl,
        load_session_messages_from_jsonl_keeping_decoded_tail, write_session_messages,
    };
    use crate::store::naming::session_meta;
    use reasonix_contract::gojson;
    use reasonix_contract::provider::Role;

    fn user_message(content: &str) -> Message {
        Message {
            role: Role::USER,
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
    fn write_session_messages_escapes_html_like_go() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");

        write_session_messages(&path, &[user_message("<b>&</b>")]).expect("write");

        let data = fs::read_to_string(&path).expect("read transcript");
        assert!(data.contains("\\u003c"));
        assert!(data.contains("\\u0026"));
        assert!(!data.contains('<'));
    }

    #[test]
    fn jsonl_decode_errors_name_the_path() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        fs::write(&path, b"{\"role\":\"user\",\"content\":\n").expect("write torn line");

        let err =
            load_session_messages_from_jsonl(&path).expect_err("torn line fails strict decode");
        assert!(err.to_string().starts_with("decode "));
        assert!(err.to_string().contains(path.to_string_lossy().as_ref()));

        let good = gojson::marshal(&user_message("kept")).expect("marshal");
        fs::write(&path, format!("{good}\n{{\"role\":\n")).expect("write valid plus torn");
        let tail = load_session_messages_from_jsonl_keeping_decoded_tail(&path);
        assert_eq!(tail.len(), 1);
    }

    #[test]
    fn save_if_absent_creates_then_refuses() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = session_of(vec![user_message("absent")]);

        session.save_if_absent(&path_str).expect("create");
        assert!(path.exists());

        let err = session.save_if_absent(&path_str).expect_err("refused");
        assert_eq!(err.to_string(), "file already exists");
    }

    #[test]
    fn superseded_transcripts_are_kept_warn_only() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        fs::write(&path, "line-one\n").expect("write transcript");

        keep_superseded_transcript(&path_str, true);

        let kept = fs::read_to_string(session_superseded(&path)).expect("superseded kept");
        assert!(kept.starts_with("// superseded at "));
        assert!(kept.contains(" by the runtime holding this session's lease\n"));
        assert!(kept.ends_with("line-one\n"));

        let other = dir.path().join("other.jsonl");
        let other_str = other.to_string_lossy().into_owned();
        fs::write(&other, "other\n").expect("write transcript");
        keep_superseded_transcript(&other_str, false);
        assert!(!session_superseded(&other).exists());
    }

    #[test]
    fn file_stamps_track_existence_and_modification() {
        let dir = tempdir();
        let missing = dir.path().join("missing.jsonl");
        assert_eq!(file_stamp(&missing), FileStampOnDisk::default());

        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = session_of(vec![user_message("stamped")]);
        session.save(&path_str).expect("save");

        let stamps = stamp_baseline(&path);
        assert!(stamps[0].here);
        assert!(stamps[1].here);
        assert!(stamps[2].here);
        assert!(!stamps[3].here);
        assert!(stamps[0].size > 0);
    }

    #[test]
    fn session_artifact_exists_scans_the_sidecars() {
        let dir = tempdir();

        let empty = dir.path().join("empty.jsonl");
        assert!(!session_artifact_exists(&empty));

        let meta_only = dir.path().join("meta-only.jsonl");
        fs::write(session_meta(&meta_only), b"{}\n").expect("write branch meta");
        assert!(session_artifact_exists(&meta_only));

        let legacy = dir.path().join("legacy.jsonl");
        fs::write(
            session_event_log(&legacy),
            "{\"schema_version\":0,\"type\":\"legacy\"}\n",
        )
        .expect("write legacy log");
        assert!(!session_artifact_exists(&legacy));

        let native = dir.path().join("native.jsonl");
        fs::write(
            session_event_log(&native),
            "{\"schema_version\":3,\"type\":\"replace\"}\n",
        )
        .expect("write native log");
        assert!(session_artifact_exists(&native));
    }
}
