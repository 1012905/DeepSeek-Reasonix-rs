//! ACP-030: `session/list` parameter rejection, sidecar beats reduction, live
//! override, cwd match, and timestamp ordering (`session.go:590-649`).

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::session_list::{
    cwd_matches, is_absolute_lexical, merge_sessions, reduce_meta_beats, sort_infos,
    validate_list_params,
};
use reasonix_acp::sidecar::AcpSessionMeta;
use reasonix_acp::wire::{SessionInfo, SessionListResult};

fn meta(id: &str, cwd: &str, updated: &str, redirect: bool) -> AcpSessionMeta {
    AcpSessionMeta {
        session_id: id.to_owned(),
        cwd: cwd.to_owned(),
        title: Some(format!("title {id}")),
        updated_at: updated.to_owned(),
        active_transcript: redirect.then(|| format!("{id}.redirect.jsonl")),
        ..Default::default()
    }
}

fn live(id: &str, cwd: &str, updated: &str) -> SessionInfo {
    SessionInfo {
        session_id: id.to_owned(),
        cwd: cwd.to_owned(),
        title: None,
        updated_at: Some(updated.to_owned()),
        meta: None,
    }
}

#[test]
fn cwd_checked_before_cursor_and_both_messages_are_verbatim() {
    let err = validate_list_params(Some("relative"), Some("cur")).unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidParams);
    assert_eq!(err.message, "session/list: cwd must be an absolute path");

    let err = validate_list_params(None, Some("cur")).unwrap_err();
    assert_eq!(err.message, "session/list: unsupported cursor");

    assert!(is_absolute_lexical("/proj"));
    assert!(is_absolute_lexical("C:\\proj"));
    assert!(!is_absolute_lexical("proj"));
}

#[test]
fn empty_body_parameters_default_to_unfiltered_first_page() {
    let filter = validate_list_params(None, None).unwrap();
    assert!(filter.is_none());
    let filter = validate_list_params(Some("  "), Some("  ")).unwrap();
    assert!(filter.is_none());
}

#[test]
fn sidecar_duplicates_reduce_by_beats_before_live_override() {
    let metas = vec![
        meta("s1", "/p", "2026-01-01T00:00:00Z", false),
        meta("s1", "/p", "2026-01-05T00:00:00Z", false),
        meta("s2", "/p", "2026-01-02T00:00:00Z", true),
        meta("s2", "/p", "2026-01-03T00:00:00Z", false),
    ];
    let reduced = reduce_meta_beats(metas);
    assert_eq!(reduced.len(), 2);
    let s1 = reduced.iter().find(|m| m.session_id == "s1").unwrap();
    assert_eq!(s1.updated_at, "2026-01-05T00:00:00Z");
    let s2 = reduced.iter().find(|m| m.session_id == "s2").unwrap();
    // Live transcript beats the redirect regardless of timestamp.
    assert!(s2.active_transcript.is_none());
}

#[test]
fn live_entries_override_persisted_by_id() {
    let metas = vec![meta("s1", "/old", "2026-01-01T00:00:00Z", false)];
    let merged = merge_sessions(
        metas,
        vec![live("s1", "/live", "2026-01-02T00:00:00Z")],
        None,
    );
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].cwd, "/live");
    assert!(merged[0].title.is_none());
}

#[test]
fn cwd_filter_compares_cleaned_paths() {
    let info = live("s1", "/proj/src", "2026-01-01T00:00:00Z");
    assert!(cwd_matches(&info, "/proj/./src"));
    assert!(!cwd_matches(&info, "/other"));

    let merged = merge_sessions(
        vec![],
        vec![
            live("s1", "/proj/src", "2026-01-01T00:00:00Z"),
            live("s2", "/other", "2026-01-01T00:00:00Z"),
        ],
        Some("/proj/./src"),
    );
    assert_eq!(merged.len(), 1);
}

#[test]
fn sort_timestamp_desc_then_id_asc() {
    let mut infos = vec![
        live("s3", "/p", "2026-01-01T00:00:00Z"),
        live("s1", "/p", "2026-01-03T00:00:00Z"),
        live("s2", "/p", "2026-01-03T00:00:00Z"),
        live("s4", "/p", ""),
    ];
    sort_infos(&mut infos);
    let ids: Vec<&str> = infos.iter().map(|i| i.session_id.as_str()).collect();
    assert_eq!(ids, vec!["s1", "s2", "s3", "s4"]);
}

#[test]
fn next_cursor_is_never_serialized() {
    let result = SessionListResult {
        sessions: vec![],
        next_cursor: None,
    };
    let value = serde_json::to_value(&result).unwrap();
    assert!(value.get("nextCursor").is_none());
}
