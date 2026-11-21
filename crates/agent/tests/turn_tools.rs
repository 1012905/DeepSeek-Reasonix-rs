//! The tool round: a streamed tool call dispatches through the registry,
//! its result pairs with the call in the transcript, and the loop re-samples
//! (Go: handleToolRound + executeBatch + executeOne).

mod support;

use std::sync::Arc;

use reasonix_agent::sink::kind;
use reasonix_tools::Registry;

use support::{
    Fixture, chunked_sse, fixture, registry_with_readers, serve_responses, text_frame,
    tool_call_frame, usage_frame,
};

#[tokio::test]
async fn a_tool_call_runs_and_the_loop_resamples() {
    let dir = tempfile::tempdir().expect("tmp");
    let file = dir.path().join("note.txt");
    std::fs::write(&file, "kernel notes").expect("write");

    let read_args = format!(
        r#"{{"path":"{}"}}"#,
        file.to_string_lossy().replace('\\', "/")
    );
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_1", "read_file", &read_args),
            &usage_frame(20, 15, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("I read the file."),
            &usage_frame(40, 6, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let Fixture {
        agent,
        recorder,
        session,
    } = fixture(&base, registry_with_readers()).await;

    let out = agent.run("read note.txt").await.expect("turn");
    assert_eq!(out.text, "I read the file.");

    // Transcript: user, assistant(tool_calls), tool(result), assistant.
    let msgs = session.snapshot();
    assert_eq!(msgs.len(), 4, "{msgs:?}");
    assert_eq!(msgs[1].tool_calls.len(), 1);
    assert_eq!(msgs[1].tool_calls[0].id, "call_1");
    assert_eq!(msgs[1].tool_calls[0].name, "read_file");
    assert_eq!(msgs[2].role.as_str(), "tool");
    assert_eq!(msgs[2].tool_call_id, "call_1");
    assert_eq!(msgs[2].name, "read_file");
    assert!(
        msgs[2].content.contains("kernel notes"),
        "{}",
        msgs[2].content
    );
    assert!(msgs[2].tool_failure.is_none());
    assert_eq!(msgs[3].content, "I read the file.");

    // Events: eager dispatch, then the result card, in that order.
    let kinds = recorder.kinds();
    let dispatch = kinds
        .iter()
        .position(|k| k == kind::TOOL_DISPATCH)
        .expect("dispatch");
    let result = kinds
        .iter()
        .position(|k| k == kind::TOOL_RESULT)
        .expect("result");
    assert!(dispatch < result, "{kinds:?}");
    let cards = recorder.events_of(kind::TOOL_RESULT);
    assert_eq!(cards.len(), 1);
    assert!(
        cards[0]
            .tool
            .as_ref()
            .expect("tool")
            .output
            .contains("kernel notes")
    );

    // Usage: one billable record per round, emitted per round.
    let usage_events = recorder.events_of(kind::USAGE);
    assert_eq!(usage_events.len(), 2, "{kinds:?}");
    assert_eq!(usage_events[0].usage.as_ref().expect("u").prompt_tokens, 20);
    assert_eq!(usage_events[1].usage.as_ref().expect("u").prompt_tokens, 40);
}

#[tokio::test]
async fn an_unknown_tool_is_refused_and_answered() {
    // The model invents a tool; the host answers with the unknown-tool
    // error and the model recovers with text (Go: parseToolCall).
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_9", "quantum_leap", r#"{}"#),
            &usage_frame(10, 4, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("I cannot do that."),
            &usage_frame(15, 5, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let Fixture {
        agent,
        recorder,
        session,
        ..
    } = fixture(&base, registry_with_readers()).await;

    let out = agent.run("leap").await.expect("turn");
    assert_eq!(out.text, "I cannot do that.");

    let msgs = session.snapshot();
    assert_eq!(msgs.len(), 4, "{msgs:?}");
    assert!(
        msgs[2].content.contains("unknown tool"),
        "{}",
        msgs[2].content
    );
    assert!(!msgs[2].tool_failure.as_ref().expect("failure").blocked);
    let cards = recorder.events_of(kind::TOOL_RESULT);
    assert_eq!(
        cards[0].tool.as_ref().expect("t").err,
        "unknown tool \"quantum_leap\""
    );
}

#[tokio::test]
async fn a_config_disabled_tool_carries_the_registry_refusal() {
    let registry = Registry::new();
    for tool in reasonix_tools::builtin::unconfined_readers() {
        registry.add(tool);
    }
    registry.add(Arc::new(reasonix_tools::builtin::WriteFileTool::default()));
    registry.disable_tools(&["write_file"]);
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(
                0,
                "call_2",
                "write_file",
                r#"{"path":"x.txt","content":"hi"}"#,
            ),
            &usage_frame(10, 4, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("understood."),
            &usage_frame(15, 5, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let Fixture { agent, session, .. } = fixture(&base, Arc::new(registry)).await;

    let out = agent.run("write").await.expect("turn");
    assert_eq!(out.text, "understood.");

    let msgs = session.snapshot();
    let tool_msg = &msgs[2];
    assert!(
        tool_msg.content.contains("blocked:"),
        "{}",
        tool_msg.content
    );
    assert!(
        tool_msg.content.contains("tool.disabled_by_config"),
        "{}",
        tool_msg.content
    );
    let failure = tool_msg.tool_failure.as_ref().expect("tool_failure");
    assert_eq!(failure.refusal_code, "tool.disabled_by_config");
    assert!(failure.blocked);
}

#[tokio::test]
async fn a_read_only_batch_runs_and_a_malformed_argument_echoes_the_schema() {
    // read_file twice in one reply: both run; a third call with broken
    // arguments reports the tool's schema (Go: the malformed-arguments
    // detail).
    let dir = tempfile::tempdir().expect("tmp");
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "alpha").expect("write");
    let args = format!(
        r#"{{"path":"{}"}}"#,
        file.to_string_lossy().replace('\\', "/")
    );

    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_a", "read_file", &args),
            &usage_frame(10, 4, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &tool_call_frame(0, "call_b", "read_file", "{\"path\": broken"),
            &usage_frame(14, 4, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("fixed it."),
            &usage_frame(20, 3, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let Fixture { agent, session, .. } = fixture(&base, registry_with_readers()).await;

    agent.run("read twice").await.expect("turn");
    let msgs = session.snapshot();
    assert_eq!(msgs.len(), 6, "{msgs:?}");
    assert!(msgs[2].content.contains("alpha"));
    let broken = &msgs[4];
    assert!(
        broken.content.contains("invalid arguments"),
        "{}",
        broken.content
    );
    // The echoed schema is what makes the retry land valid; the tool name
    // rides the pairing, not the schema.
    assert!(
        broken.content.contains("\"type\":\"object\""),
        "{}",
        broken.content
    );
}
