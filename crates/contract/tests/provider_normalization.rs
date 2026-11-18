//! Normalization behavior ports (Go: 'provider_test.go',
//! 'pairing_probe_test.go'; the ModelMessages/ProjectionMessages legs of
//! those files belong to the projection slice). Contract:
//! docs/contracts/provider-message.md §7.

use std::borrow::Cow;

use reasonix_contract::provider::{
    DecisionReceipt, INTERRUPTED_TOOL_RESULT, LOCAL_ONLY_TOOL_ID, LOCAL_ONLY_TOOL_NAME, Message,
    Role, ToolCall, args_need_repair, attach_standalone_decision_receipts,
    backfill_tool_call_names, close_truncated_json, id_distinct, needs_tool_call_arg_repair,
    normalize_messages, normalize_session_messages, pair_tool_results, repair_tool_call_args,
    sanitize_tool_pairing, session_tool_results, tool_turn_well_formed,
};

fn user(content: &str) -> Message {
    Message {
        role: Role::USER,
        content: content.to_owned(),
        ..Message::default()
    }
}

fn system(content: &str) -> Message {
    Message {
        role: Role::SYSTEM,
        content: content.to_owned(),
        ..Message::default()
    }
}

fn assistant_calls(calls: Vec<ToolCall>) -> Message {
    Message {
        role: Role::ASSISTANT,
        tool_calls: calls,
        ..Message::default()
    }
}

fn call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: id.to_owned(),
        name: name.to_owned(),
        ..ToolCall::default()
    }
}

fn call_args(id: &str, name: &str, arguments: &str) -> ToolCall {
    ToolCall {
        arguments: arguments.to_owned(),
        ..call(id, name)
    }
}

fn tool_result(id: &str, name: &str, content: &str) -> Message {
    Message {
        role: Role::TOOL,
        tool_call_id: id.to_owned(),
        name: name.to_owned(),
        content: content.to_owned(),
        ..Message::default()
    }
}

fn receipt(id: &str) -> DecisionReceipt {
    DecisionReceipt {
        id: id.to_owned(),
        kind: "tool".to_owned(),
        tool: "bash".to_owned(),
        subject: "session".to_owned(),
        outcome: "allow_once".to_owned(),
        via: None,
    }
}

#[test]
fn sanitize_backfills_dangling_call() {
    let in_ = vec![
        user("list files"),
        assistant_calls(vec![call("c1", "ls")]),
        user("never mind"),
    ];
    let out = sanitize_tool_pairing(&in_);
    assert_eq!(out.len(), 4);
    assert_eq!(out[2].role, Role::TOOL);
    assert_eq!(out[2].tool_call_id, "c1");
    assert_eq!(out[2].name, "ls");
    assert_eq!(out[2].content, INTERRUPTED_TOOL_RESULT);
    assert_eq!(out[1].tool_calls[0].name, "ls");
    assert_eq!(out[1].tool_calls[0].arguments, "{}");
    // The caller's history is never mutated: the repaired copy carries the
    // object arguments while the stored call keeps its empty string.
    assert_eq!(in_[1].tool_calls[0].name, "ls");
    assert_eq!(in_[1].tool_calls[0].arguments, "");
    // Every assistant call is answered by some tool result.
    let answered: Vec<&str> = out
        .iter()
        .filter(|m| m.role == Role::TOOL)
        .map(|m| m.tool_call_id.as_str())
        .collect();
    for m in out.iter() {
        for c in &m.tool_calls {
            assert!(
                answered.contains(&c.id.as_str()),
                "call {} unanswered",
                c.id
            );
        }
    }
}

#[test]
fn sanitize_keeps_call_order_and_multiple() {
    let in_ = vec![
        assistant_calls(vec![call("a", ""), call("b", ""), call("c", "")]),
        tool_result("b", "", "B"),
        tool_result("a", "", "A"),
    ];
    let out = sanitize_tool_pairing(&in_);
    assert_eq!(out.len(), 4);
    let call_ids: Vec<&str> = out[0].tool_calls.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(call_ids, vec!["a", "b", "c"]);
    for c in &out[0].tool_calls {
        assert_eq!(c.arguments, "{}");
    }
    assert_eq!(out[1].tool_call_id, "a");
    assert_eq!(out[1].content, "A");
    assert_eq!(out[2].tool_call_id, "b");
    assert_eq!(out[2].content, "B");
    assert_eq!(out[3].tool_call_id, "c");
    assert_eq!(out[3].content, INTERRUPTED_TOOL_RESULT);
}

#[test]
fn sanitize_drops_orphan_tool_message() {
    let in_ = vec![
        user("hi"),
        tool_result("ghost", "bash", "leftover"),
        Message {
            role: Role::ASSISTANT,
            content: "hello".to_owned(),
            ..Message::default()
        },
    ];
    let out = sanitize_tool_pairing(&in_);
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|m| m.role != Role::TOOL));
    assert_eq!(out[1].content, "hello");
}

#[test]
fn sanitize_leaves_well_formed_unchanged() {
    let in_ = vec![
        system("sys"),
        user("q"),
        assistant_calls(vec![call_args("c1", "ls", "{}")]),
        tool_result("c1", "ls", "main.go"),
        Message {
            role: Role::ASSISTANT,
            content: "done".to_owned(),
            ..Message::default()
        },
    ];
    let out = sanitize_tool_pairing(&in_);
    assert!(matches!(out, Cow::Borrowed(_)));
    assert_eq!(out.len(), in_.len());
    assert_eq!(out[0].role, Role::SYSTEM);
    assert_eq!(out[2].tool_calls[0].arguments, "{}");
    assert_eq!(out[3].content, "main.go");
    assert_eq!(out[4].content, "done");
}

#[test]
fn sanitize_drops_local_only_interrupted_output() {
    let local = Message {
        role: Role::TOOL,
        tool_call_id: LOCAL_ONLY_TOOL_ID.to_owned(),
        name: LOCAL_ONLY_TOOL_NAME.to_owned(),
        content: "partial answer".to_owned(),
        reasoning_content: "partial reasoning".to_owned(),
        tool_calls: vec![call("partial", "write_file")],
        local_only: true,
        ..Message::default()
    };
    let in_ = vec![user("task"), local, user("continue")];
    let wire = sanitize_tool_pairing(&in_);
    assert_eq!(wire.len(), 2);
    assert_eq!(wire[0].content, "task");
    assert_eq!(wire[1].content, "continue");
}

#[test]
fn session_normalization_preserves_local_only_display() {
    let local = Message {
        role: Role::TOOL,
        tool_call_id: LOCAL_ONLY_TOOL_ID.to_owned(),
        name: LOCAL_ONLY_TOOL_NAME.to_owned(),
        content: "partial answer".to_owned(),
        reasoning_content: "partial reasoning".to_owned(),
        tool_calls: vec![call("partial", "write_file")],
        local_only: true,
        ..Message::default()
    };
    let in_ = vec![user("task"), local, user("continue")];
    let session = normalize_session_messages(&in_);
    assert_eq!(session.len(), 3);
    assert!(session[1].local_only);
    assert_eq!(session[1].content, "partial answer");
    assert_eq!(session[1].reasoning_content, "partial reasoning");
}

#[test]
fn attached_decision_receipt_positional_pairing() {
    let stored = vec![
        user("run it"),
        Message {
            role: Role::ASSISTANT,
            tool_calls: vec![call_args("", "bash", "{}")],
            decision_receipts: vec![Some(receipt("approval-1"))],
            ..Message::default()
        },
        tool_result("", "bash", "ok"),
    ];
    // Current encoding: sanitize keeps stored receipts untouched; stripping
    // them for the provider wire belongs to the projection slice.
    let out = sanitize_tool_pairing(&stored);
    assert_eq!(out.len(), 3);
    assert_eq!(out[1].decision_receipts, vec![Some(receipt("approval-1"))]);
    // Legacy encoding: receipt already stripped; the empty call id falls
    // back to positional pairing and the recorded result survives.
    let mut legacy = stored.clone();
    legacy[1].decision_receipts = Vec::new();
    let out = sanitize_tool_pairing(&legacy);
    assert_eq!(out.len(), 3);
    assert_eq!(out[1].role, Role::ASSISTANT);
    assert_eq!(out[2].role, Role::TOOL);
    assert_eq!(out[2].content, "ok");
}

#[test]
fn session_normalization_migrates_interleaved_decision_receipt() {
    let msgs = vec![
        user("run it"),
        assistant_calls(vec![call_args("call-1", "bash", "{}")]),
        Message {
            role: Role::ASSISTANT,
            local_only: true,
            decision_receipt: Some(receipt("approval-1")),
            ..Message::default()
        },
        tool_result("call-1", "bash", "actual result"),
    ];
    let got = normalize_session_messages(&msgs);
    assert_eq!(got.len(), 3);
    assert_eq!(got[1].decision_receipts, vec![Some(receipt("approval-1"))]);
    assert_eq!(got[2].role, Role::TOOL);
    assert_eq!(got[2].content, "actual result");
    assert_ne!(got[2].content, INTERRUPTED_TOOL_RESULT);
}

#[test]
fn local_only_sentinel_is_safe_for_legacy_reader() {
    let legacy_view = vec![
        user("task"),
        assistant_calls(vec![call_args("c1", "read_file", "{}")]),
        tool_result("c1", "read_file", "ok"),
        Message {
            role: Role::TOOL,
            tool_call_id: LOCAL_ONLY_TOOL_ID.to_owned(),
            name: LOCAL_ONLY_TOOL_NAME.to_owned(),
            content: "partial reasoning that must not leak".to_owned(),
            local_only: true,
            ..Message::default()
        },
        user("continue"),
    ];
    let wire = sanitize_tool_pairing(&legacy_view);
    assert_eq!(wire.len(), 4);
    for m in wire.iter() {
        assert_ne!(m.tool_call_id, LOCAL_ONLY_TOOL_ID);
        assert!(!m.content.contains("must not leak"));
    }
}

#[test]
fn session_normalization_preserves_standalone_tool_message() {
    let in_ = vec![
        system("sys"),
        user("run it"),
        tool_result("c1", "bash", "large output"),
    ];
    let out = normalize_session_messages(&in_);
    assert!(matches!(out, Cow::Borrowed(_)));
    assert_eq!(out.len(), in_.len());
    assert_eq!(out[2].role, Role::TOOL);
    assert_eq!(out[2].content, "large output");
}

#[test]
fn session_normalization_preserves_extra_tool_result() {
    let in_ = vec![
        assistant_calls(vec![call("c1", "bash")]),
        tool_result("c1", "bash", "ok"),
        tool_result("ghost", "bash", "saved extra output"),
    ];
    let out = normalize_session_messages(&in_);
    assert_eq!(out.len(), 3);
    assert_eq!(out[2].tool_call_id, "ghost");
    assert_eq!(out[2].content, "saved extra output");
    let wire = sanitize_tool_pairing(&in_);
    assert_eq!(wire.len(), 2);
}

#[test]
fn sanitize_closes_truncated_args() {
    let cases: &[(&str, &str)] = &[
        ("{", "{}"),
        ("{\"time\": 2", "{\"time\": 2}"),
        ("{\"command\": \"ls -la", "{\"command\": \"ls -la\"}"),
        ("{\"a\": 1,", "{\"a\": 1}"),
        ("{\"a\":", "{\"a\":null}"),
        ("{\"path\": \"C:\\\\tmp\\", "{\"path\": \"C:\\\\tmp\"}"),
        ("{\"items\": [1, 2", "{\"items\": [1, 2]}"),
        ("total garbage", "{}"),
        ("{\"ok\": true}", "{\"ok\": true}"),
        ("", "{}"),
        (" \n", "{}"),
    ];
    for (args_in, want) in cases {
        let in_ = vec![
            assistant_calls(vec![call_args("c1", "bash", args_in)]),
            tool_result("c1", "bash", "r"),
        ];
        let out = sanitize_tool_pairing(&in_);
        assert_eq!(out[0].tool_calls[0].arguments, *want, "case {args_in:?}");
        assert_eq!(
            in_[0].tool_calls[0].arguments, *args_in,
            "history mutated for {args_in:?}"
        );
    }
}

#[test]
fn backfill_by_id() {
    let calls = vec![call("c1", ""), call("c2", "grep")];
    let results = vec![tool_result("c2", "grep", ""), tool_result("c1", "ls", "")];
    let out = backfill_tool_call_names(&calls, &results);
    assert_eq!(out[0].name, "ls");
    assert_eq!(out[1].name, "grep");
    assert_eq!(calls[0].name, "");
}

#[test]
fn backfill_positional() {
    let calls = vec![call("", ""), call("", "")];
    let results = vec![tool_result("", "ls", ""), tool_result("", "cat", "")];
    let out = backfill_tool_call_names(&calls, &results);
    assert_eq!(out[0].name, "ls");
    assert_eq!(out[1].name, "cat");
}

#[test]
fn backfill_unpaired_stays_empty() {
    let calls = vec![call("c1", "")];
    let out = backfill_tool_call_names(&calls, &[]);
    assert_eq!(out[0].name, "");
}

#[test]
fn backfill_no_empty_returns_input() {
    let calls = vec![call("c1", "ls"), call("c2", "grep")];
    let results = vec![tool_result("c1", "x", "")];
    let out = backfill_tool_call_names(&calls, &results);
    assert!(matches!(out, Cow::Borrowed(_)));
    assert_eq!(out.len(), 2);
}

#[test]
fn sanitize_backfills_empty_name() {
    let in_ = vec![
        assistant_calls(vec![call("c1", "")]),
        tool_result("c1", "ls", "main.go"),
    ];
    let out = sanitize_tool_pairing(&in_);
    assert_eq!(out[0].tool_calls[0].name, "ls");
    assert_eq!(out[0].tool_calls[0].arguments, "{}");
    assert_eq!(out[1].name, "ls");
    assert_eq!(out[1].content, "main.go");
    assert_eq!(in_[0].tool_calls[0].name, "");
}

#[test]
fn sanitize_backfills_missing_tool_result_name() {
    let in_ = vec![
        assistant_calls(vec![call("c1", "ls")]),
        Message {
            role: Role::TOOL,
            tool_call_id: "c1".to_owned(),
            content: "main.go".to_owned(),
            ..Message::default()
        },
    ];
    let out = sanitize_tool_pairing(&in_);
    assert_eq!(out[1].name, "ls");
    assert_eq!(in_[1].name, "");
}

#[test]
fn session_normalization_keeps_empty_arguments() {
    let in_ = vec![
        assistant_calls(vec![call_args("c1", "status", "")]),
        tool_result("c1", "status", "ok"),
    ];
    let out = normalize_session_messages(&in_);
    assert!(matches!(out, Cow::Borrowed(_)));
    assert_eq!(out[0].tool_calls[0].arguments, "");
    assert_eq!(out[1].content, "ok");
}

#[test]
fn duplicate_tool_call_ids_keep_each_result() {
    let msgs = vec![
        user("go"),
        assistant_calls(vec![call("dup", "read_file"), call("dup", "grep")]),
        tool_result("dup", "read_file", "FILE-RESULT"),
        tool_result("dup", "grep", "GREP-RESULT"),
    ];
    let out = sanitize_tool_pairing(&msgs);
    assert_eq!(out.len(), 4);
    assert_eq!(out[2].content, "FILE-RESULT");
    assert_eq!(out[2].name, "read_file");
    assert_eq!(out[3].content, "GREP-RESULT");
    assert_eq!(out[3].name, "grep");
}

#[test]
fn empty_tool_call_ids_keep_each_result() {
    let msgs = vec![
        assistant_calls(vec![call("", "a"), call("", "b")]),
        tool_result("", "a", "RESULT-A"),
        tool_result("", "b", "RESULT-B"),
    ];
    let out = sanitize_tool_pairing(&msgs);
    let contents: Vec<&str> = out
        .iter()
        .filter(|m| m.role == Role::TOOL)
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(contents.len(), 2);
    assert_ne!(contents[0], contents[1]);
    assert_eq!(contents[0], "RESULT-A");
    assert_eq!(contents[1], "RESULT-B");
}

#[test]
fn attach_standalone_decision_receipts_fast_path_borrows() {
    let healthy = vec![
        user("q"),
        assistant_calls(vec![call("c1", "bash")]),
        tool_result("c1", "bash", "ok"),
    ];
    let out = attach_standalone_decision_receipts(&healthy);
    assert!(matches!(out, Cow::Borrowed(_)));
    assert_eq!(out.len(), healthy.len());
}

#[test]
fn attach_standalone_decision_receipts_folds_and_resets() {
    let msgs = vec![
        user("go"),
        Message {
            role: Role::ASSISTANT,
            content: "a1".to_owned(),
            ..Message::default()
        },
        Message {
            role: Role::ASSISTANT,
            local_only: true,
            decision_receipt: Some(receipt("r1")),
            ..Message::default()
        },
        user("next"),
        Message {
            role: Role::ASSISTANT,
            local_only: true,
            decision_receipt: Some(receipt("r2")),
            ..Message::default()
        },
    ];
    let out = attach_standalone_decision_receipts(&msgs);
    assert_eq!(out.len(), 4);
    assert_eq!(out[1].decision_receipts, vec![Some(receipt("r1"))]);
    assert_eq!(out[3].decision_receipts.len(), 0);
    assert!(out[3].local_only);
    assert_eq!(out[3].decision_receipt, Some(receipt("r2")));
}

#[test]
fn pair_tool_results_by_id_and_placeholder() {
    let calls = vec![call("a", "ls"), call("b", "cat"), call("c", "grep")];
    let avail = vec![tool_result("b", "cat", "B"), tool_result("a", "ls", "A")];
    let out = pair_tool_results(&calls, &avail);
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].tool_call_id, "a");
    assert_eq!(out[0].name, "ls");
    assert_eq!(out[0].content, "A");
    assert_eq!(out[1].tool_call_id, "b");
    assert_eq!(out[1].content, "B");
    assert_eq!(out[2].tool_call_id, "c");
    assert_eq!(out[2].name, "grep");
    assert_eq!(out[2].content, INTERRUPTED_TOOL_RESULT);
}

#[test]
fn pair_tool_results_positional_overwrites_identity() {
    let calls = vec![call("a", "ls"), call("", "cat")];
    let avail = vec![tool_result("zzz", "other", "X"), tool_result("q", "r", "Y")];
    let out = pair_tool_results(&calls, &avail);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].tool_call_id, "a");
    assert_eq!(out[0].name, "ls");
    assert_eq!(out[0].content, "X");
    assert_eq!(out[1].tool_call_id, "");
    assert_eq!(out[1].name, "cat");
    let calls2 = vec![call("a", "ls"), call("b", "cat"), call("d", "grep")];
    let out2 = pair_tool_results(&calls2, &avail);
    assert_eq!(out2.len(), 3);
    assert_eq!(out2[2].tool_call_id, "d");
    assert_eq!(out2[2].content, INTERRUPTED_TOOL_RESULT);
}

#[test]
fn session_tool_results_appends_placeholders_only() {
    let calls = vec![call("a", "ls"), call("b", "cat")];
    let avail = vec![tool_result("b", "cat", "B"), tool_result("ghost", "x", "G")];
    let out = session_tool_results(&calls, &avail);
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].content, "B");
    assert_eq!(out[1].content, "G");
    assert_eq!(out[2].tool_call_id, "a");
    assert_eq!(out[2].name, "ls");
    assert_eq!(out[2].content, INTERRUPTED_TOOL_RESULT);
    let calls2 = vec![call("", "a"), call("", "b"), call("", "c")];
    let avail2 = vec![tool_result("", "a", "A")];
    let out2 = session_tool_results(&calls2, &avail2);
    assert_eq!(out2.len(), 3);
    assert_eq!(out2[0].content, "A");
    assert_eq!(out2[1].content, INTERRUPTED_TOOL_RESULT);
    assert_eq!(out2[2].content, INTERRUPTED_TOOL_RESULT);
}

#[test]
fn close_truncated_json_repairs_streams() {
    assert_eq!(close_truncated_json("{"), "{}");
    assert_eq!(close_truncated_json("{\"time\": 2"), "{\"time\": 2}");
    assert_eq!(
        close_truncated_json("{\"command\": \"ls -la"),
        "{\"command\": \"ls -la\"}"
    );
    assert_eq!(close_truncated_json("{\"a\": 1,"), "{\"a\": 1}");
    assert_eq!(close_truncated_json("{\"a\":"), "{\"a\":null}");
    assert_eq!(
        close_truncated_json("{\"path\": \"C:\\\\tmp\\"),
        "{\"path\": \"C:\\\\tmp\"}"
    );
    assert_eq!(
        close_truncated_json("{\"items\": [1, 2"),
        "{\"items\": [1, 2]}"
    );
    assert_eq!(close_truncated_json("total garbage"), "{}");
    assert_eq!(close_truncated_json("{\"ok\": true}"), "{\"ok\": true}");
    assert_eq!(close_truncated_json(""), "{}");
    assert_eq!(close_truncated_json(" \n"), "{}");
    assert_eq!(
        close_truncated_json("{\"a\": [{\"b\": 1, \"c\": "),
        "{\"a\": [{\"b\": 1, \"c\":null}]}"
    );
}

#[test]
fn args_need_repair_modes() {
    assert!(args_need_repair("", true));
    assert!(!args_need_repair("", false));
    assert!(args_need_repair(" \n", true));
    assert!(!args_need_repair(" \n", false));
    assert!(!args_need_repair("{\"ok\": true}", true));
    assert!(!args_need_repair("{\"ok\": true}", false));
    assert!(args_need_repair("{bad", true));
    assert!(args_need_repair("{bad", false));
    let empty = vec![call_args("c", "t", "")];
    assert!(!needs_tool_call_arg_repair(&empty, false));
    assert!(needs_tool_call_arg_repair(&empty, true));
    let valid = vec![call_args("c", "t", "{}")];
    assert!(!needs_tool_call_arg_repair(&valid, true));
}

#[test]
fn repair_tool_call_args_copy_on_write() {
    let m = assistant_calls(vec![call_args("c1", "bash", "{\"a\": 1")]);
    let repaired = repair_tool_call_args(m.clone(), true);
    assert_eq!(repaired.tool_calls[0].arguments, "{\"a\": 1}");
    assert_eq!(m.tool_calls[0].arguments, "{\"a\": 1");
    let empty = assistant_calls(vec![call_args("c1", "bash", "")]);
    let repaired = repair_tool_call_args(empty.clone(), true);
    assert_eq!(repaired.tool_calls[0].arguments, "{}");
    let kept = repair_tool_call_args(empty, false);
    assert_eq!(kept.tool_calls[0].arguments, "");
    let valid = assistant_calls(vec![call_args("c1", "bash", "{}")]);
    let out = repair_tool_call_args(valid, true);
    assert_eq!(out.tool_calls[0].arguments, "{}");
}

#[test]
fn well_formed_and_id_distinct_primitives() {
    let calls = vec![call("c1", "ls")];
    let results = vec![tool_result("c1", "ls", "r")];
    assert!(tool_turn_well_formed(&calls, &results));
    assert!(!tool_turn_well_formed(&calls, &[]));
    let wrong_name = vec![tool_result("c1", "other", "r")];
    assert!(!tool_turn_well_formed(&calls, &wrong_name));
    let empty_call_name = vec![call("c1", "")];
    let matching = vec![tool_result("c1", "", "r")];
    assert!(!tool_turn_well_formed(&empty_call_name, &matching));
    let unnamed = vec![Message {
        role: Role::TOOL,
        tool_call_id: "c1".to_owned(),
        ..Message::default()
    }];
    assert!(!tool_turn_well_formed(&calls, &unnamed));
    let distinct = vec![call("a", "x")];
    assert!(id_distinct(&distinct));
    let empty_id = vec![call("", "x")];
    assert!(!id_distinct(&empty_id));
    let dup = vec![call("x", "a"), call("x", "b")];
    assert!(!id_distinct(&dup));
    assert!(id_distinct(&[]));
}

#[test]
fn fast_path_returns_borrowed_only_when_untouched() {
    let healthy = vec![
        assistant_calls(vec![call_args("c1", "ls", "{}")]),
        tool_result("c1", "ls", "ok"),
    ];
    assert!(matches!(normalize_messages(&healthy), Cow::Borrowed(_)));
    assert!(matches!(
        normalize_session_messages(&healthy),
        Cow::Borrowed(_)
    ));
    let empty_args = vec![
        assistant_calls(vec![call_args("c1", "ls", "")]),
        tool_result("c1", "ls", "ok"),
    ];
    assert!(matches!(normalize_messages(&empty_args), Cow::Owned(_)));
    assert!(matches!(
        normalize_session_messages(&empty_args),
        Cow::Borrowed(_)
    ));
    let orphan = vec![tool_result("ghost", "x", "g")];
    assert!(matches!(normalize_messages(&orphan), Cow::Owned(_)));
    assert!(matches!(
        normalize_session_messages(&orphan),
        Cow::Borrowed(_)
    ));
    let local = vec![
        user("q"),
        Message {
            role: Role::ASSISTANT,
            local_only: true,
            ..Message::default()
        },
        user("go"),
    ];
    assert!(matches!(normalize_messages(&local), Cow::Owned(_)));
    assert!(matches!(
        normalize_session_messages(&local),
        Cow::Borrowed(_)
    ));
}
