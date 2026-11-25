use std::cell::Cell;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use reasonix_contract::provider::{Message, Role, ToolCall};
use serde_json::Value;

use super::materialize::read_dag_payloads;
use super::replay::{DagState, replay_session_dag};
use super::{
    DagError, dag_transcript_matches, load_session_dag_messages,
    load_session_dag_messages_with_limits, load_session_dag_tail,
    load_session_dag_tail_with_limits,
};
use crate::sessionstore::eventlog::{SESSION_REPLAY_MAX_PAYLOAD_BYTES, SessionReplayLimits};
use crate::store::session_event_log;

fn tempdir() -> tempfile::TempDir {
    let base = option_env!("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("reasonix-dag-")
        .tempdir_in(base)
        .unwrap()
}

fn record(fields: &str) -> String {
    format!(r#"{{"schema_version":2,"at":"2026-09-25T13:00:00Z",{fields}}}"#)
}

fn record_at(fields: &str, at: &str) -> String {
    format!(r#"{{"schema_version":2,"at":"{at}",{fields}}}"#)
}

fn payload(role: &str, content: &str) -> String {
    format!(
        r#"[{{"role":{},"content":{}}}]"#,
        Value::String(role.to_owned()),
        Value::String(content.to_owned())
    )
}

fn message(id: &str, parent: &str, head: &str, role: &str, content: &str) -> String {
    record(&format!(
        r#""type":"message","id":"{id}","parent":"{parent}","head":"{head}","msgs":{}"#,
        payload(role, content)
    ))
}

fn raw_message(id: &str, parent: &str, head: &str, msgs: &str) -> String {
    record(&format!(
        r#""type":"message","id":"{id}","parent":"{parent}","head":"{head}","msgs":{msgs}"#
    ))
}

fn write_dag(dir: &Path, records: &[String]) -> PathBuf {
    let session = dir.join("20260925-130000.000000000-deepseek-flash.jsonl");
    fs::write(&session, b"").unwrap();
    let mut body = records.join("\n");
    body.push('\n');
    fs::write(session_event_log(&session), body).unwrap();
    session
}

fn base() -> Vec<String> {
    vec![
        record(r#""type":"log","generation":1"#),
        record(r#""type":"writer","writer":"w1","pid":42"#),
        message("m1", "", "main", "system", "sys"),
        message("m2", "m1", "main", "user", "<session-context>"),
        message("m3", "m2", "main", "user", "hello"),
        message("m4", "m3", "main", "assistant", "hi"),
        record(r#""type":"turn_begin","head":"main""#),
        record(r#""type":"turn_end","head":"main""#),
    ]
}

fn contents(msgs: &[Message]) -> Vec<String> {
    msgs.iter()
        .map(|m| format!("{}:{}", m.role, m.content))
        .collect()
}

fn load(session: &Path) -> (Vec<Message>, bool) {
    load_session_dag_messages(session).unwrap()
}

fn assert_limit<T: std::fmt::Debug>(result: Result<T, DagError>, resource: &str) {
    match result.unwrap_err() {
        DagError::Limit(err) => assert_eq!(err.resource, resource),
        other => panic!("expected {resource} limit, got {other:?}"),
    }
}

#[test]
fn dag_loads_selected_head_and_host_origin() {
    let dir = tempdir();
    let records = vec![
        message("m1", "", "", "system", "sys"),
        raw_message(
            "m2",
            "m1",
            "",
            r#"[{"role":"user","content":"host context","origin":"host"}]"#,
        ),
        raw_message(
            "m3",
            "m2",
            "",
            r#"[{"role":"user","content":"hello","origin":"user"}]"#,
        ),
        message("m4", "m3", "", "assistant", "hi"),
    ];
    let (msgs, damaged) = load(&write_dag(dir.path(), &records));
    assert!(!damaged);
    assert_eq!(
        contents(&msgs),
        [
            "system:sys",
            "user:host context",
            "user:hello",
            "assistant:hi"
        ]
    );
    assert!(msgs[1].host_authored);
    assert!(!msgs[2].host_authored);
}

#[test]
fn dag_invalid_origin_is_a_hard_io_error() {
    let dir = tempdir();
    let session = write_dag(
        dir.path(),
        &[raw_message(
            "m1",
            "",
            "main",
            r#"[{"role":"user","content":"hello","origin":42}]"#,
        )],
    );
    assert!(matches!(
        load_session_dag_messages(&session),
        Err(DagError::Io(_))
    ));
}

#[test]
fn dag_system_overlay_prepends_when_the_chain_has_no_system_message() {
    let dir = tempdir();
    let session = write_dag(
        dir.path(),
        &[
            message("u", "", "main", "user", "hello"),
            record(r#""type":"system","head":"main","msgs":[{"role":"system","content":"sys"}]"#),
        ],
    );
    let (msgs, damaged) = load(&session);
    assert!(!damaged);
    assert_eq!(contents(&msgs), ["system:sys", "user:hello"]);
}

#[test]
fn dag_explicit_select_prefers_the_last_live_head() {
    let dir = tempdir();
    let mut records = base();
    records.push(record(
        r#""type":"fork","head":"main","new_head":"h2","from":"m4""#,
    ));
    records.push(message("m5", "m4", "h2", "assistant", "forked"));
    records.push(record(r#""type":"select","head":"h2""#));
    records.push(record(r#""type":"select","head":"main""#));
    let (msgs, damaged) = load(&write_dag(dir.path(), &records));
    assert!(!damaged);
    assert_eq!(msgs.last().unwrap().content, "hi");

    records.push(record(r#""type":"select","head":"h2""#));
    records.push(record(r#""type":"select","head":"undeclared""#));
    let (msgs, damaged) = load(&write_dag(dir.path(), &records));
    assert!(!damaged);
    assert_eq!(msgs.last().unwrap().content, "forked");
}

#[test]
fn dag_activity_orders_heads_and_offset_breaks_ties() {
    let dir = tempdir();
    let mut records = base();
    records.push(record_at(
        r#""type":"fork","head":"main","new_head":"h2","from":"m4""#,
        "2026-09-25T14:00:00Z",
    ));
    records.push(record_at(
        r#""type":"message","id":"m5","parent":"m4","head":"h2","msgs":[{"role":"assistant","content":"newer activity"}]"#,
        "2026-09-25T14:00:00Z",
    ));
    records.push(record_at(
        r#""type":"message","id":"m6","parent":"m4","head":"main","msgs":[{"role":"assistant","content":"later offset"}]"#,
        "2026-09-25T13:00:00Z",
    ));
    let (msgs, _) = load(&write_dag(dir.path(), &records));
    assert_eq!(msgs.last().unwrap().content, "newer activity");

    records.push(record_at(
        r#""type":"message","id":"m7","parent":"m4","head":"main","msgs":[{"role":"assistant","content":"tie winner"}]"#,
        "2026-09-25T14:00:00Z",
    ));
    let (msgs, _) = load(&write_dag(dir.path(), &records));
    assert_eq!(msgs.last().unwrap().content, "tie winner");
}

#[test]
fn dag_retired_heads_fall_back_only_when_no_live_head_exists() {
    let dir = tempdir();
    let mut records = base();
    records.push(record(
        r#""type":"fork","head":"main","new_head":"h2","from":"m4""#,
    ));
    records.push(message("m5", "m4", "h2", "assistant", "forked"));
    records.push(record(r#""type":"select","head":"h2""#));
    records.push(record(r#""type":"retire","head":"h2""#));
    let (msgs, _) = load(&write_dag(dir.path(), &records));
    assert_eq!(msgs.last().unwrap().content, "hi");

    records.push(record(r#""type":"retire","head":"main""#));
    let (msgs, _) = load(&write_dag(dir.path(), &records));
    assert_eq!(msgs.last().unwrap().content, "forked");
}

#[test]
fn dag_fork_snapshots_system_and_rewind_moves_the_leaf() {
    let dir = tempdir();
    let mut records = base();
    records.push(record(
        r#""type":"system","head":"main","msgs":[{"role":"system","content":"inherited"}]"#,
    ));
    records.push(record(
        r#""type":"fork","head":"main","new_head":"h2","from":"m4""#,
    ));
    records.push(record(
        r#""type":"system","head":"main","msgs":[{"role":"system","content":"main only"}]"#,
    ));
    records.push(record(r#""type":"select","head":"h2""#));
    let (msgs, _) = load(&write_dag(dir.path(), &records));
    assert_eq!(
        contents(&msgs),
        [
            "system:inherited",
            "user:<session-context>",
            "user:hello",
            "assistant:hi"
        ]
    );

    records.push(record(r#""type":"rewind","head":"h2","to":"m2""#));
    records.push(message("m5", "m2", "h2", "assistant", "after rewind"));
    records.push(record(r#""type":"rewind","head":"h2","to":"m3""#));
    let (msgs, _) = load(&write_dag(dir.path(), &records));
    assert_eq!(msgs.last().unwrap().content, "hello");
}

#[test]
fn dag_patch_redaction_and_system_overlays_keep_their_precedence() {
    let dir = tempdir();
    let mut records = base();
    records.push(record(
        r#""type":"patch","target":"m3","msgs":[{"role":"user","content":"patch one"}]"#,
    ));
    records.push(record(
        r#""type":"redact","targets":{"m3":[{"role":"user","content":"secret"}],"m4":[{"role":"assistant","content":"redacted"}]}"#,
    ));
    records.push(record(
        r#""type":"patch","target":"m3","msgs":[{"role":"user","content":"patch two"}]"#,
    ));
    records.push(record(
        r#""type":"patch","target":"m4","msgs":[{"role":"assistant","content":"patch three"}]"#,
    ));
    records.push(record(
        r#""type":"system","head":"main","msgs":[{"role":"system","content":"sys2"}]"#,
    ));
    let (msgs, damaged) = load(&write_dag(dir.path(), &records));
    assert!(!damaged);
    assert_eq!(
        contents(&msgs),
        [
            "system:sys2",
            "user:<session-context>",
            "user:secret",
            "assistant:redacted"
        ]
    );
}

#[test]
fn dag_duplicate_ids_and_unknown_patch_targets_are_tolerated() {
    let dir = tempdir();
    let mut records = base();
    records.push(message("m4", "m1", "other", "assistant", "duplicate"));
    records.push(record(
        r#""type":"patch","target":"unknown","msgs":[{"role":"user","content":42}]"#,
    ));
    let (msgs, damaged) = load(&write_dag(dir.path(), &records));
    assert!(!damaged);
    assert_eq!(
        contents(&msgs),
        [
            "system:sys",
            "user:<session-context>",
            "user:hello",
            "assistant:hi"
        ]
    );
}

#[test]
fn dag_malformed_duplicate_and_off_path_payloads_damage_replay() {
    for extra in [
        raw_message(
            "m4",
            "m1",
            "other",
            r#"[{"role":"assistant","content":42}]"#,
        ),
        raw_message("bad", "", "other", r#"[{"role":"user","content":42}]"#),
    ] {
        let dir = tempdir();
        let mut records = base();
        records.push(extra);
        let (msgs, damaged) = load(&write_dag(dir.path(), &records));
        assert!(damaged);
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs.last().unwrap().content, "hi");
    }
}

#[test]
fn dag_unknown_entry_and_schema_are_hard_errors() {
    let dir = tempdir();
    let mut records = base();
    records.push(record(r#""type":"future_marker""#));
    match load_session_dag_messages_with_limits(
        &write_dag(dir.path(), &records),
        &SessionReplayLimits::default(),
    )
    .unwrap_err()
    {
        DagError::UnsupportedEntry { entry_type, .. } => assert_eq!(entry_type, "future_marker"),
        other => panic!("unexpected error: {other:?}"),
    }

    let dir = tempdir();
    let records = [r#"{"schema_version":3,"type":"log"}"#.to_owned()];
    match load_session_dag_messages_with_limits(
        &write_dag(dir.path(), &records),
        &SessionReplayLimits::default(),
    )
    .unwrap_err()
    {
        DagError::UnsupportedSchema { version, .. } => assert_eq!(version, 3),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn dag_bare_null_is_an_unsupported_zero_schema() {
    let dir = tempdir();
    let session = write_dag(dir.path(), &["null".to_owned()]);
    match load_session_dag_messages(&session).unwrap_err() {
        DagError::UnsupportedSchema { version, .. } => assert_eq!(version, 0),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn dag_torn_tail_preserves_the_valid_prefix() {
    let dir = tempdir();
    let mut records = base();
    records.push(r#"{"schema_version":2,"type":"message","id":"m9""#.to_owned());
    let (msgs, damaged) = load(&write_dag(dir.path(), &records));
    assert!(damaged);
    assert_eq!(msgs.len(), 4);
    assert_eq!(msgs.last().unwrap().content, "hi");
}

#[test]
fn dag_cycles_and_missing_parents_materialize_safely() {
    let cases: Vec<(Vec<String>, Vec<&str>)> = vec![
        (
            vec![message("self", "self", "main", "user", "self")],
            vec!["user:self"],
        ),
        (
            vec![
                message("a", "b", "main", "user", "a"),
                message("b", "a", "main", "assistant", "b"),
            ],
            vec!["user:a", "assistant:b"],
        ),
        (
            vec![message("child", "missing", "main", "user", "child")],
            vec!["user:child"],
        ),
    ];
    for (records, expected) in cases {
        let dir = tempdir();
        let (msgs, damaged) = load(&write_dag(dir.path(), &records));
        assert!(!damaged);
        assert_eq!(contents(&msgs), expected);
    }
}

#[test]
fn dag_tail_budget_preserves_system_and_cuts_at_a_user_boundary() {
    let dir = tempdir();
    let records = vec![
        message("s", "", "main", "system", "sys"),
        message("u0", "s", "main", "user", "zero"),
        message("a0", "u0", "main", "assistant", "answer zero"),
        message("a1", "a0", "main", "assistant", "answer one"),
        message("u1", "a1", "main", "user", "one"),
        message("a2", "u1", "main", "assistant", "answer two"),
    ];
    let session = write_dag(dir.path(), &records);
    let limits = SessionReplayLimits {
        tail_budget: (payload("system", "sys").len()
            + payload("assistant", "answer one").len()
            + payload("user", "one").len()
            + payload("assistant", "answer two").len()) as i64,
        ..SessionReplayLimits::default()
    };
    let tail = load_session_dag_tail_with_limits(&session, &limits).unwrap();
    assert_eq!(tail.dropped, 3);
    assert_eq!(
        contents(&tail.messages),
        ["system:sys", "user:one", "assistant:answer two"]
    );

    let full = load_session_dag_tail(&session).unwrap();
    assert_eq!(full.dropped, 0);
    assert_eq!(full.messages.len(), records.len());
}

#[test]
fn dag_tail_budget_never_drops_the_final_payload() {
    let dir = tempdir();
    let session = write_dag(
        dir.path(),
        &[message("only", "", "main", "user", &"x".repeat(200))],
    );
    let limits = SessionReplayLimits {
        tail_budget: 1,
        ..SessionReplayLimits::default()
    };
    let tail = load_session_dag_tail_with_limits(&session, &limits).unwrap();
    assert_eq!(tail.dropped, 0);
    assert_eq!(tail.messages.len(), 1);
}

#[test]
fn dag_payload_locations_follow_go_json_field_semantics() {
    let cases = [
        (
            r#"{"schema_version":2,"type":"redact","targets":{"m3":[{"role":"user","content":"first"}]},"TARGETS":{"m4":[{"role":"assistant","content":"four"}],"m3":[{"role":"user","content":"last"}]}}"#,
            "user:last",
            "assistant:four",
        ),
        (
            r#"{"schema_version":2,"type":"redact","targets":{"m3":[{"role":"user","content":"first"}]},"targets":null,"targets":{"m4":[{"role":"assistant","content":"four"}]}}"#,
            "user:hello",
            "assistant:four",
        ),
        (
            r#"{"schema_version":2,"type":"patch","target":"m3","ignored":{"nested":[{"text":"中文 \\\" { [ ] }"}]},"m\u0073gs":[{"role":"user","content":"中文 \\ \" { [ ] }"}]}"#,
            "user:中文 \\ \" { [ ] }",
            "assistant:hi",
        ),
        (
            r#"{"schema_version":2,"type":"patch","target":"m3","MSGS":[{"role":"user","content":"upper"}]}"#,
            "user:upper",
            "assistant:hi",
        ),
        (
            r#"{"schema_version":2,"type":"redact","targets":{"m\u0033":[{"role":"user","content":"escaped target"}]}}"#,
            "user:escaped target",
            "assistant:hi",
        ),
    ];
    for (extra, user, assistant) in cases {
        let dir = tempdir();
        let mut records = base();
        records.push(extra.to_owned());
        let (msgs, damaged) = load(&write_dag(dir.path(), &records));
        assert!(!damaged, "{extra}");
        assert_eq!(
            msgs[2].content,
            user.strip_prefix("user:").unwrap(),
            "{extra}"
        );
        assert_eq!(
            msgs[3].content,
            assistant.strip_prefix("assistant:").unwrap(),
            "{extra}"
        );
    }
}

#[test]
fn dag_replay_budgets_have_typed_resource_identities() {
    let dir = tempdir();
    let session = write_dag(dir.path(), &base());
    let limits = SessionReplayLimits {
        max_bytes: 1,
        ..SessionReplayLimits::default()
    };
    assert_limit(
        load_session_dag_messages_with_limits(&session, &limits),
        "encoded_bytes",
    );

    let limits = SessionReplayLimits {
        max_records: 0,
        ..SessionReplayLimits::default()
    };
    assert_limit(
        load_session_dag_messages_with_limits(&session, &limits),
        "event_records",
    );

    let limits = SessionReplayLimits {
        max_messages: 0,
        ..SessionReplayLimits::default()
    };
    assert_limit(
        load_session_dag_messages_with_limits(&session, &limits),
        "messages",
    );

    let limits = SessionReplayLimits {
        max_payload_bytes: 1,
        ..SessionReplayLimits::default()
    };
    assert_limit(
        load_session_dag_messages_with_limits(&session, &limits),
        "message_payload_bytes",
    );

    let dir = tempdir();
    let session = write_dag(
        dir.path(),
        &[raw_message(
            "m1",
            "",
            "main",
            r#"[{"role":"assistant","content":"","tool_calls":[{"id":"c1","name":"n","arguments":"{}"}]}]"#,
        )],
    );
    let limits = SessionReplayLimits {
        max_collection_items: 0,
        ..SessionReplayLimits::default()
    };
    assert_limit(
        load_session_dag_messages_with_limits(&session, &limits),
        "message_collection_items",
    );
}

#[test]
fn dag_metadata_budget_is_fixed_and_typed() {
    let mut state = DagState::new("log");
    state
        .charge_metadata(SESSION_REPLAY_MAX_PAYLOAD_BYTES)
        .unwrap();
    match state.charge_metadata(1).unwrap_err() {
        DagError::Limit(err) => {
            assert_eq!(err.resource, "dag_metadata_bytes");
            assert_eq!(err.value, SESSION_REPLAY_MAX_PAYLOAD_BYTES + 1);
            assert_eq!(err.limit, SESSION_REPLAY_MAX_PAYLOAD_BYTES);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn dag_missing_log_is_an_io_error() {
    let dir = tempdir();
    let session = write_dag(dir.path(), &base());
    fs::remove_file(session_event_log(&session)).unwrap();
    assert!(matches!(
        load_session_dag_messages(&session),
        Err(DagError::Io(_))
    ));
}

#[test]
fn dag_transcript_comparison_ignores_only_the_leading_system() {
    let dir = tempdir();
    let session = write_dag(dir.path(), &base());
    let (disk, _) = load(&session);
    let mut snapshot = disk.clone();
    snapshot[0].content = "this build's prompt".to_owned();
    snapshot[1].created_at = 123;
    assert!(dag_transcript_matches(&session, &snapshot));
    assert!(dag_transcript_matches(&session, &disk[1..]));

    snapshot[2].content = "edited".to_owned();
    assert!(!dag_transcript_matches(&session, &snapshot));

    let mut later_system = disk;
    later_system.push(Message {
        role: Role::SYSTEM,
        content: "late system".to_owned(),
        ..Message::default()
    });
    assert!(!dag_transcript_matches(&session, &later_system));
}

#[test]
fn dag_transcript_comparison_migrates_legacy_provider_content() {
    let dir = tempdir();
    let session = write_dag(
        dir.path(),
        &[
            message("s", "", "main", "system", "sys"),
            raw_message(
                "u",
                "s",
                "main",
                r#"[{"role":"user","content":"display","provider_content":"provider"}]"#,
            ),
        ],
    );
    let snapshot = [
        Message {
            role: Role::SYSTEM,
            content: "new prompt".to_owned(),
            ..Message::default()
        },
        Message {
            role: Role::USER,
            content: "provider".to_owned(),
            raw_content: "display".to_owned(),
            ..Message::default()
        },
    ];
    assert!(dag_transcript_matches(&session, &snapshot));
}

#[test]
fn dag_transcript_comparison_normalizes_historical_tool_turns() {
    let dir = tempdir();
    let session = write_dag(
        dir.path(),
        &[
            message("s", "", "main", "system", "sys"),
            message("u", "s", "main", "user", "search"),
            raw_message(
                "a",
                "u",
                "main",
                r#"[{"role":"assistant","content":"","tool_calls":[{"id":"c1","name":"","arguments":"{}"}]}]"#,
            ),
            raw_message(
                "t",
                "a",
                "main",
                r#"[{"role":"tool","tool_call_id":"c1","name":"search","content":"ok"}]"#,
            ),
        ],
    );
    let snapshot = [
        Message {
            role: Role::SYSTEM,
            content: "new prompt".to_owned(),
            ..Message::default()
        },
        Message {
            role: Role::USER,
            content: "search".to_owned(),
            ..Message::default()
        },
        Message {
            role: Role::ASSISTANT,
            tool_calls: vec![ToolCall {
                id: "c1".to_owned(),
                name: "search".to_owned(),
                arguments: "{}".to_owned(),
                ..ToolCall::default()
            }],
            ..Message::default()
        },
        Message {
            role: Role::TOOL,
            tool_call_id: "c1".to_owned(),
            name: "search".to_owned(),
            content: "ok".to_owned(),
            ..Message::default()
        },
    ];
    assert!(dag_transcript_matches(&session, &snapshot));
}

struct CountingReader<R> {
    inner: R,
    bytes: Cell<usize>,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes.set(self.bytes.get() + n);
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for CountingReader<R> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(position)
    }
}

#[test]
fn dag_multi_target_redaction_reads_each_payload_range_once() {
    let dir = tempdir();
    let mut records = Vec::new();
    let mut targets = Vec::new();
    for i in 0..256 {
        let id = format!("m{i}");
        let parent = if i == 0 {
            String::new()
        } else {
            format!("m{}", i - 1)
        };
        records.push(message(
            &id,
            &parent,
            "main",
            "user",
            &format!("original {i}"),
        ));
        targets.push(format!(
            r#""{id}":[{{"role":"user","content":"removed {i}"}}]"#
        ));
    }
    records.push(record(&format!(
        r#""type":"redact","targets":{{{}}}"#,
        targets.join(",")
    )));
    let session = write_dag(dir.path(), &records);
    let log = session_event_log(&session);
    let log_size = fs::metadata(&log).unwrap().len();
    let mut file = File::open(&log).unwrap();
    let limits = SessionReplayLimits::default();
    let state = replay_session_dag(&mut file, &log, &limits).unwrap();
    let selected = state.selected_head();
    let (refs, dropped) = state.chain(&selected, &limits).unwrap();
    assert_eq!(dropped, 0);
    let mut reader = CountingReader {
        inner: &file,
        bytes: Cell::new(0),
    };
    let msgs = read_dag_payloads(&mut reader, &log.to_string_lossy(), &refs, &limits).unwrap();
    assert_eq!(msgs.len(), 256);
    assert_eq!(msgs[255].content, "removed 255");
    assert!(reader.bytes.get() <= log_size as usize * 2);
}

#[test]
fn dag_payload_truncation_between_passes_is_a_hard_io_error() {
    let dir = tempdir();
    let session = write_dag(dir.path(), &base());
    let log = session_event_log(&session);
    let mut file = File::open(&log).unwrap();
    let limits = SessionReplayLimits::default();
    let state = replay_session_dag(&mut file, &log, &limits).unwrap();
    let selected = state.selected_head();
    let (refs, _) = state.chain(&selected, &limits).unwrap();
    drop(file);
    let truncator = fs::OpenOptions::new().write(true).open(&log).unwrap();
    truncator.set_len(0).unwrap();
    drop(truncator);
    let mut file = File::open(&log).unwrap();
    assert!(matches!(
        read_dag_payloads(&mut file, &log.to_string_lossy(), &refs, &limits),
        Err(DagError::Io(_))
    ));
}
