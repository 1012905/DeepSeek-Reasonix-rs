
use std::fs::{self, File};
use std::sync::RwLock;
use std::time::{Duration, SystemTime};

use reasonix_contract::provider::Role;

use super::*;
use crate::sessionstore::branch::{BRANCH_META_COUNTS_VERSION, load_branch_meta};
use crate::sessionstore::eventlog::read_session_event_index;
use crate::sessionstore::eventlog::replay_session_event_log;
use crate::sessionstore::eventlog::test_support::tempdir;
use crate::sessionstore::session::{
    RecoveryBranchOptions, SessionState, load_session, load_session_messages, load_session_unlocked,
};
use crate::sessionstore::session_lease::{
    next_session_write_generation, try_acquire_session_lease,
};
use crate::store::naming::{session_event_log, session_meta};
use reasonix_contract::gojson;
use reasonix_contract::provider::ToolCall;
use std::sync::Arc;

fn user_message(content: &str) -> Message {
    Message {
        role: Role::USER,
        content: content.to_string(),
        ..Message::default()
    }
}

fn assistant_message(content: &str) -> Message {
    Message {
        role: Role::ASSISTANT,
        content: content.to_string(),
        ..Message::default()
    }
}

fn session_of(messages: Vec<Message>) -> Session {
    Session {
        state: RwLock::new(SessionState {
            messages,
            ..SessionState::default()
        }),
    }
}

fn transcript_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .expect("read transcript")
        .lines()
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect()
}

#[test]
fn save_writes_transcript_log_ledger_and_index() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = session_of(vec![user_message("hello world")]);

    session.save(&path_str).expect("save");

    let lines = transcript_lines(&path);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("\"role\":\"user\""));
    let replay = replay_session_event_log(&session_event_log(&path)).expect("replay");
    assert_eq!(replay.msgs.len(), 1);
    let meta = load_branch_meta(&path)
        .expect("branch meta io")
        .expect("branch meta present");
    assert_eq!(meta.revision, 1);
    assert!(!meta.content_digest.is_empty());
    assert_eq!(meta.preview, "hello world");
    assert_eq!(meta.turns, 1);
    assert_eq!(meta.schema_version, BRANCH_META_COUNTS_VERSION);
    let index = read_session_event_index(&path)
        .expect("index io")
        .expect("index present");
    assert_eq!(index.message_count, 1);
    assert_eq!(index.revision, 1);
    assert!(session.snapshot_up_to_date(&path_str));
    let persisted = session
        .persisted_state(&path_str)
        .expect("persisted baseline");
    assert_eq!(persisted.revision, 1);
}

#[test]
fn unchanged_save_is_a_fast_path_no_op() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = session_of(vec![user_message("steady")]);
    session.save(&path_str).expect("first save");
    let log_before = fs::read(session_event_log(&path)).expect("read log");
    let transcript_before = fs::read(&path).expect("read transcript");

    session.save(&path_str).expect("second save");

    assert_eq!(
        fs::read(session_event_log(&path)).expect("read log"),
        log_before
    );
    assert_eq!(fs::read(&path).expect("read transcript"), transcript_before);
    let meta = load_branch_meta(&path)
        .expect("branch meta io")
        .expect("branch meta present");
    assert_eq!(meta.revision, 1);
}

#[test]
fn append_extends_the_log_and_bumps_the_revision() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = session_of(vec![user_message("one")]);
    session.save(&path_str).expect("first save");

    session.add(assistant_message("hi there"));
    session.save(&path_str).expect("second save");

    assert_eq!(transcript_lines(&path).len(), 2);
    let replay = replay_session_event_log(&session_event_log(&path)).expect("replay");
    assert_eq!(replay.msgs.len(), 2);
    let meta = load_branch_meta(&path)
        .expect("branch meta io")
        .expect("branch meta present");
    assert_eq!(meta.revision, 2);
    let index = read_session_event_index(&path)
        .expect("index io")
        .expect("index present");
    assert_eq!(index.message_count, 2);
    assert_eq!(index.revision, 2);
}

#[test]
fn fast_baseline_decision_tracks_the_armed_stamp() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = session_of(vec![user_message("fast")]);
    session.save(&path_str).expect("save");

    assert_eq!(session.disk_prefix_len(&path_str), 1);
    let msgs = session.snapshot();
    let (_, _, prefix) =
        digest_size_and_prefix(&msgs, session.disk_prefix_len(&path_str)).expect("digest");
    let decision = session
        .fast_snapshot_decision(&path_str, &msgs, prefix)
        .expect("fast decision");
    assert!(decision.up_to_date);
    assert!(!decision.append_only);

    File::options()
        .write(true)
        .open(session_meta(&path))
        .expect("open branch meta")
        .set_modified(SystemTime::now() + Duration::from_secs(3600))
        .expect("stamp branch meta mtime");
    assert!(
        session
            .fast_snapshot_decision(&path_str, &msgs, prefix)
            .is_none()
    );
}

#[test]
fn diverged_save_is_refused() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let first = session_of(vec![user_message("hello world")]);
    first.save(&path_str).expect("first save");

    let divergent = session_of(vec![user_message("a different turn")]);
    let err = divergent
        .save(&path_str)
        .expect_err("diverged save refused");
    let conflict = match err {
        SaveError::Conflict(conflict) => conflict,
        other => panic!("expected a snapshot conflict, got: {other}"),
    };
    assert_eq!(conflict.kind, SessionSnapshotConflictKind::Diverged);
    assert_eq!(conflict.existing_messages, 1);
    assert_eq!(conflict.snapshot_messages, 1);
    assert_eq!(conflict.base_revision, 0);
    assert_eq!(conflict.disk_revision, 1);
    assert_eq!(conflict.diverged_at, 0);
    assert_eq!(conflict.disk_role, "user");
    assert_eq!(conflict.snapshot_role, "user");
    assert!(conflict.to_string().contains(
        "diverged on disk (1 messages, revision 1) from snapshot (1 messages, revision 0)"
    ));
}

#[test]
fn stale_prefix_save_is_refused() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let first = session_of(vec![user_message("hello world")]);
    first.save(&path_str).expect("first save");

    let second = session_of(vec![
        user_message("hello world"),
        assistant_message("more one"),
        assistant_message("more two"),
    ]);
    second.save(&path_str).expect("second save");

    first.add(assistant_message("more one"));
    let err = first.save(&path_str).expect_err("stale save refused");
    let conflict = match err {
        SaveError::Conflict(conflict) => conflict,
        other => panic!("expected a snapshot conflict, got: {other}"),
    };
    assert_eq!(conflict.kind, SessionSnapshotConflictKind::StalePrefix);
    assert_eq!(conflict.diverged_at, -1);
    assert!(
        conflict.to_string().contains(
            "has 3 messages at revision 2; stale snapshot has 2 messages from revision 1"
        )
    );
}

#[test]
fn future_schema_log_refuses_the_save() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    fs::write(
        session_event_log(&path),
        "{\"schema_version\":99,\"type\":\"replace\"}\n",
    )
    .expect("write future log");

    let session = session_of(vec![user_message("hello")]);
    let err = session.save(&path_str).expect_err("future schema refused");
    assert!(
        err.to_string()
            .contains("uses schema 99; this build supports up to 3")
    );
}

#[test]
fn dag_log_refuses_the_save_as_foreign() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    fs::write(
        session_event_log(&path),
        "{\"schema_version\":2,\"type\":\"replace\"}\n",
    )
    .expect("write dag log");

    let session = session_of(vec![user_message("hello")]);
    let err = session.save(&path_str).expect_err("dag refused");
    assert!(
        err.to_string()
            .contains("session event log is written by Reasonix 1.x")
    );
}

fn write_dag(session: &Path, user_content: &str) -> Vec<u8> {
    fs::write(session, b"").expect("write checkpoint");
    let records = [
            format!(
                "{{\"schema_version\":2,\"at\":\"2026-09-25T13:00:00Z\",\"type\":\"message\",\"id\":\"u\",\"parent\":\"\",\"head\":\"main\",\"msgs\":[{{\"role\":\"user\",\"content\":{}}}]}}",
                serde_json::to_string(user_content).expect("content JSON")
            ),
            "{\"schema_version\":2,\"at\":\"2026-09-25T13:00:01Z\",\"type\":\"message\",\"id\":\"a\",\"parent\":\"u\",\"head\":\"main\",\"msgs\":[{\"role\":\"assistant\",\"content\":\"answer\"}]}".to_string(),
        ];
    let body = format!("{}\n", records.join("\n"));
    fs::write(session_event_log(session), body.as_bytes()).expect("write DAG");
    body.into_bytes()
}

#[test]
fn unchanged_foreign_dag_save_is_typed_and_writes_nothing() {
    let dir = tempdir();
    let path = dir.path().join("foreign.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let before = write_dag(&path, "same question");

    let session = load_session(&path_str).expect("load foreign DAG");
    let err = session
        .save(&path_str)
        .expect_err("unchanged foreign save refused");
    assert!(matches!(
        err,
        SaveError::DagUnchanged { ref path } if path == &path_str
    ));
    assert_eq!(fs::read(session_event_log(&path)).expect("log"), before);
    assert_eq!(fs::read(&path).expect("checkpoint"), b"");
}

#[test]
fn changed_foreign_dag_save_is_refused_without_writing_a_sidecar() {
    let dir = tempdir();
    let path = dir.path().join("foreign.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let before = write_dag(&path, "stored question");

    let session = session_of(vec![user_message("different question")]);
    let err = session
        .save(&path_str)
        .expect_err("changed foreign save refused");
    assert!(matches!(
        err,
        SaveError::DagForeign { ref path } if path == &path_str
    ));
    assert_eq!(fs::read(session_event_log(&path)).expect("log"), before);
    assert!(!session_meta(&path).exists());
}

#[test]
fn an_unknown_record_opens_but_refuses_a_rewrite() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = session_of(vec![user_message("hello")]);
    session.save(&path_str).expect("first save");

    // A newer build appended a record this one does not understand.
    let log = session_event_log(&path);
    let mut body = fs::read_to_string(&log).expect("read log");
    body.push_str(
            "{\"schema_version\":1,\"type\":\"mutate\",\"messages\":[{\"role\":\"user\",\"content\":\"future\"}]}
",
        );
    fs::write(&log, &body).expect("append unknown record");

    // The read tolerates it and keeps the known transcript.
    let loaded = load_session(&path_str).expect("load");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded.snapshot()[0].content, "hello");

    // A fold would drop it, so it is refused.
    let grown = session_of(loaded.snapshot());
    grown.add(assistant_message("answer"));
    let err = grown
        .save_rewrite_compact(&path_str)
        .expect_err("fold refused");
    match &err {
        SaveError::UnknownEventType { path, event_type } => {
            assert!(path.ends_with("s.jsonl"));
            assert_eq!(event_type, "mutate");
        }
        other => panic!("expected an unknown-event-type refusal, got: {other}"),
    }
    assert_eq!(fs::read_to_string(&log).expect("unread log"), body);

    // An append still lands: the record stays in place.
    let appended = session_of(loaded.snapshot());
    appended.add(assistant_message("answer"));
    appended.save(&path_str).expect("append allowed");
    let replay = replay_session_event_log(&log).expect("replay");
    assert_eq!(replay.unknown_records, vec!["mutate".to_string()]);
    assert_eq!(replay.msgs.len(), 2);
}

#[test]
fn the_display_index_tracks_every_save() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = session_of(vec![user_message("one")]);
    session.save(&path_str).expect("first save");

    let index_path = session_display_index(&path);
    let first = load_session_display_index(&index_path).expect("index");
    assert_eq!(first.message_count, 1);
    assert_eq!(first.authored_turns, 1);
    assert!(first.revision_known);

    session.add(assistant_message("two"));
    session.add(user_message("three"));
    session.save(&path_str).expect("append save");
    let second = load_session_display_index(&index_path).expect("index");
    assert_eq!(second.message_count, 3);
    assert_eq!(second.authored_turns, 2);
    assert_eq!(second.revision, first.revision + 1);
    // The prefix entries survive the incremental extension unchanged.
    assert_eq!(second.entries[0], first.entries[0]);
    let transcript = fs::metadata(&path).expect("stat").len() as i64;
    assert_eq!(second.transcript_size, transcript);
}

#[test]
fn the_read_model_append_keeps_the_transcript_current() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = session_of(vec![user_message("one")]);
    session.save(&path_str).expect("first save");
    let before =
        crate::sessionstore::session::load_session_messages_from_jsonl(&path).expect("model");
    assert_eq!(before.len(), 1);

    session.add(assistant_message("two"));
    session.save(&path_str).expect("append save");
    let after =
        crate::sessionstore::session::load_session_messages_from_jsonl(&path).expect("model");
    assert_eq!(after.len(), 2);
    assert_eq!(after[1].content, "two");
}

#[test]
fn plain_jsonl_load_anchors_the_baseline() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let line = gojson::marshal(&user_message("plain")).expect("marshal");
    fs::write(&path, format!("{line}\n")).expect("write transcript");

    let session = load_session_unlocked(&path).expect("load");
    assert_eq!(session.snapshot().len(), 1);
    let persisted = session
        .persisted_state(&path_str)
        .expect("persisted baseline");
    assert_eq!(persisted.revision, 0);
    assert!(persisted.revision_known);
    assert!(!session.snapshot_up_to_date(&path_str));

    session.save(&path_str).expect("save adopts the baseline");
    let meta = load_branch_meta(&path)
        .expect("branch meta io")
        .expect("branch meta present");
    assert_eq!(meta.revision, 0);
    assert!(!session_event_log(&path).exists());
}

#[test]
fn heal_rebuilds_an_empty_checkpoint_from_the_log() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = session_of(vec![user_message("heal me")]);
    session.save(&path_str).expect("save");
    fs::write(&path, b"").expect("truncate transcript");

    let fresh = session_of(vec![user_message("heal me")]);
    fresh.save(&path_str).expect("heal and save");

    assert_eq!(transcript_lines(&path).len(), 1);
}

#[test]
fn empty_paths_are_rejected() {
    let session = session_of(vec![user_message("nope")]);

    assert_eq!(
        session.save("").expect_err("empty path").to_string(),
        "empty session path"
    );
    assert_eq!(
        session.save("   ").expect_err("blank path").to_string(),
        "empty session path"
    );
    assert_eq!(
        session
            .save_if_absent(" ")
            .expect_err("blank path")
            .to_string(),
        "empty session path"
    );
}

#[test]
fn rewrite_mode_rewrites_the_log_history() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = session_of(vec![user_message("first")]);
    session.save(&path_str).expect("first save");

    session.add(assistant_message("second"));
    session.save_rewrite(&path_str).expect("rewrite save");

    let replay = replay_session_event_log(&session_event_log(&path)).expect("replay");
    assert_eq!(replay.msgs.len(), 2);
    let meta = load_branch_meta(&path)
        .expect("branch meta io")
        .expect("branch meta present");
    assert_eq!(meta.revision, 2);
    assert_eq!(transcript_lines(&path).len(), 2);
}

const RESHAPED_DIFF: &str = "@@ -1 +1 @@\n-a\n+b\n";

fn recovery_jsonl(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read session dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.contains("-recovery-")
                && name.ends_with(".jsonl")
                && !name.ends_with(".events.jsonl")
        })
        .collect();
    names.sort();
    names
}

fn assistant_message_with_call(content: &str, diff: &str) -> Message {
    let mut message = assistant_message(content);
    message.tool_calls.push(ToolCall {
        id: "call_1".to_string(),
        name: "edit".to_string(),
        arguments: r#"{"path":"f"}"#.to_string(),
        diff: diff.to_string(),
        added: if diff.is_empty() { 0 } else { 1 },
        removed: if diff.is_empty() { 0 } else { 1 },
        ..ToolCall::default()
    });
    message
}

#[test]
fn streaming_append_does_not_diverge() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = Session::new("");
    session.add(user_message("hello"));
    let mut hi = assistant_message("hi");
    hi.reasoning_content = "think".to_string();
    session.add(hi);
    session.save(&path_str).expect("save");
    for i in 0..20 {
        session.add(user_message(&format!("u{i}")));
        let mut call = assistant_message(&format!("a{i}"));
        call.reasoning_content = format!("r{i}");
        call.tool_calls.push(ToolCall {
            id: format!("c{i}"),
            name: "bash".to_string(),
            arguments: r#"{"cmd":"true"}"#.to_string(),
            ..ToolCall::default()
        });
        session.add(call);
        session.add(Message {
            role: Role::TOOL,
            tool_call_id: format!("c{i}"),
            name: "bash".to_string(),
            content: "ok".to_string(),
            work_duration_ms: i64::from(i),
            ..Message::default()
        });
        let mut done = assistant_message(&format!("done{i}"));
        done.reasoning_content = "more".to_string();
        session.add(done);
        session.save(&path_str).expect("save");
    }
    assert!(recovery_jsonl(dir.path()).is_empty());
}

#[test]
fn lease_held_same_revision_allows_reshaped_prefix() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = Session::new("");
    session.add(user_message("edit"));
    session.add(assistant_message_with_call("editing", ""));
    session.save(&path_str).expect("save");

    let lease = try_acquire_session_lease(&path_str).expect("lease");
    let authority = lease
        .issue_write_authority(next_session_write_generation())
        .expect("authority");
    session.bind_write_authority(Some(Arc::new(authority.clone())));

    let foreign = vec![
        user_message("edit"),
        assistant_message_with_call("editing", RESHAPED_DIFF),
    ];
    let foreign_digest = digest_session_messages(&foreign).expect("digest");
    let (revision, _) = session_content_revision(&path).expect("revision");
    append_session_replace_event(&path, &foreign, &foreign_digest, revision, "snapshot")
        .expect("append foreign replace");
    write_session_messages(&path, &foreign).expect("reshape disk");

    assert!(session.update_tool_call_preview(&ToolCall {
        id: "call_1".to_string(),
        diff: RESHAPED_DIFF.to_string(),
        added: 1,
        removed: 1,
        ..ToolCall::default()
    }));
    session.add(Message {
        role: Role::TOOL,
        tool_call_id: "call_1".to_string(),
        name: "edit".to_string(),
        content: "ok".to_string(),
        ..Message::default()
    });
    let mut done = assistant_message("done");
    done.reasoning_content = "r".to_string();
    session.add(done);
    session.save(&path_str).expect("save under authority");
    assert!(recovery_jsonl(dir.path()).is_empty());

    for i in 0..10 {
        session.add(user_message(&format!("n{i}")));
        session.add(assistant_message(&format!("r{i}")));
        session.save(&path_str).expect("save");
    }
    assert!(recovery_jsonl(dir.path()).is_empty());
}

#[test]
fn chain_after_recovery_fork_does_not_cascade() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = Session::new("");
    session.add(user_message("start"));
    session.add(assistant_message("ok"));
    session.save(&path_str).expect("save");
    session.add(user_message("unsaved"));
    session.add(assistant_message("local only tail"));
    let info = session
        .save_recovery_branch(RecoveryBranchOptions {
            original_path: path_str.clone(),
            reason: "snapshot conflict".to_string(),
            ..RecoveryBranchOptions::default()
        })
        .expect("recovery branch");
    let branch_str = info.path.to_string_lossy().into_owned();
    for i in 0..15 {
        session.add(user_message(&format!("u{i}")));
        let mut assistant = assistant_message(&format!("a{i}"));
        assistant.reasoning_content = "x".to_string();
        session.add(assistant);
        session.save(&branch_str).expect("save branch");
    }
    assert_eq!(recovery_jsonl(dir.path()).len(), 1);
}

#[test]
fn rejects_interrupted_foreign_write_without_lease() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = Session::new("");
    session.add(user_message("base"));
    session.save(&path_str).expect("save");

    let stale = load_session(&path_str).expect("reload");
    let (revision, _) = session_content_revision(&path).expect("revision");
    let mut foreign = stale.snapshot();
    foreign.push(assistant_message("foreign writer tail"));
    let foreign_digest = digest_session_messages(&foreign).expect("digest");
    append_session_replace_event(&path, &foreign, &foreign_digest, revision, "snapshot")
        .expect("append foreign replace");
    write_session_messages(&path, &foreign).expect("foreign tail");

    stale.add(assistant_message("stale writer tail"));
    let err = stale.save(&path_str).expect_err("stale writer refused");
    assert!(matches!(
        &err,
        SaveError::Conflict(conflict) if conflict.kind == SessionSnapshotConflictKind::Diverged
    ));
}

#[test]
fn authority_missing_returns_typed_error() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = Session::new("");
    session.add(user_message("u"));
    session.add(assistant_message("a"));
    session.save(&path_str).expect("save");
    session.require_write_authority();
    session.clear_write_authority();
    session.add(user_message("more"));
    let err = session.save(&path_str).expect_err("missing authority");
    assert!(matches!(
        err,
        SaveError::WriteAuthority(WriteAuthorityError::Missing)
    ));
    assert!(recovery_jsonl(dir.path()).is_empty());
}

#[test]
fn stale_authority_refused() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = Session::new("");
    session.add(user_message("u"));
    session.add(assistant_message("a"));
    session.save(&path_str).expect("save");

    let lease = try_acquire_session_lease(&path_str).expect("lease");
    let old_authority = lease
        .issue_write_authority(next_session_write_generation())
        .expect("old authority");
    session.bind_write_authority(Some(Arc::new(old_authority.clone())));
    let new_authority = lease
        .issue_write_authority(next_session_write_generation())
        .expect("new authority");
    assert!(new_authority.valid());

    session.add(user_message("more"));
    session.bind_write_authority(Some(Arc::new(old_authority)));
    let err = session.save(&path_str).expect_err("stale authority");
    assert!(matches!(
        err,
        SaveError::WriteAuthority(WriteAuthorityError::Stale)
    ));
    assert!(recovery_jsonl(dir.path()).is_empty());
}

#[test]
fn zero_byte_checkpoint_heals_from_wal() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let session = Session::new("");
    session.add(user_message("u0"));
    session.add(assistant_message("a0"));
    session.save(&path_str).expect("save");

    let lease = try_acquire_session_lease(&path_str).expect("lease");
    let authority = lease
        .issue_write_authority(next_session_write_generation())
        .expect("authority");
    session.bind_write_authority(Some(Arc::new(authority.clone())));

    fs::write(&path, b"").expect("truncate checkpoint");
    for i in 1..=100 {
        session.add(user_message(&format!("u{i}")));
        session.add(assistant_message(&format!("a{i}")));
        session.save(&path_str).expect("autosave");
    }
    assert!(recovery_jsonl(dir.path()).is_empty());
    assert!(fs::metadata(&path).expect("stat checkpoint").len() > 0);
}

#[test]
fn lease_held_without_baseline_rewrites_instead_of_forking() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let base = Session::new("sys");
    base.add(user_message("ask"));
    base.add(assistant_message_with_call("answering", ""));
    base.save(&path_str).expect("save base");

    let rebuilt = Session::new("sys");
    rebuilt.add(user_message("ask"));
    rebuilt.add(assistant_message_with_call("answering", RESHAPED_DIFF));

    let lease = try_acquire_session_lease(&path_str).expect("lease");
    let authority = lease
        .issue_write_authority(next_session_write_generation())
        .expect("authority");
    rebuilt.bind_write_authority(Some(Arc::new(authority.clone())));

    rebuilt.save(&path_str).expect("rewrite under authority");

    let back = load_session(&path_str).expect("reload");
    let got = back.snapshot();
    assert!(got.len() >= 3);
    assert!(!got[2].tool_calls.is_empty());
    assert_ne!(got[2].tool_calls[0].diff, "");
    let (raw, _from_events, _damaged) = load_session_messages(&path).expect("raw load");
    let (_, ledger) = session_content_revision(&path).expect("ledger");
    let digest = digest_session_messages(&raw).expect("digest");
    assert_eq!(ledger, digest_string(&digest));
}

#[test]
fn without_lease_still_refuses_a_reshape() {
    let dir = tempdir();
    let path = dir.path().join("s.jsonl");
    let path_str = path.to_string_lossy().into_owned();
    let base = Session::new("sys");
    base.add(user_message("ask"));
    base.add(assistant_message_with_call("answering", ""));
    base.save(&path_str).expect("save base");

    let rebuilt = Session::new("sys");
    rebuilt.add(user_message("ask"));
    rebuilt.add(assistant_message_with_call("answering", RESHAPED_DIFF));

    let err = rebuilt.save(&path_str).expect_err("reshape refused");
    assert!(matches!(
        &err,
        SaveError::Conflict(conflict) if conflict.kind == SessionSnapshotConflictKind::Diverged
    ));
}
