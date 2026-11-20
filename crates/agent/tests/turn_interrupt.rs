//! Stream interruption: a cut stream replays the frozen request; a
//! malformed payload and an auth wall are terminal. Failed attempts commit
//! nothing (Go: `streamWithSamplingRecovery` + `handleSamplingError`).

mod support;

use reasonix_agent::sink::kind;

use support::{
    Fixture, chunked_sse, fixture, registry_with_readers, serve_responses, text_frame,
    tool_call_frame, usage_frame,
};

/// A data frame that is not JSON at all (Go: StreamDecodeError).
fn garbage_frame() -> String {
    "this is not json".to_owned()
}

/// A 200 whose body ends before any terminal the protocol accepts: the
/// proxy idle-close shape (Go: premature_eof, #3953).
fn premature_close() -> Vec<u8> {
    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
        .to_vec()
}

#[tokio::test]
async fn a_malformed_payload_is_terminal_and_commits_nothing() {
    let base = serve_responses(vec![chunked_sse(&[
        &text_frame("half a"),
        &garbage_frame(),
        "[DONE]",
    ])])
    .await;
    let Fixture {
        agent,
        recorder,
        session,
        ..
    } = fixture(&base, registry_with_readers()).await;

    let err = agent.run("try").await.expect_err("decode");
    match err {
        reasonix_agent::TurnError::Provider(reasonix_provider::ProviderError::StreamDecode {
            ..
        }) => {}
        other => panic!("unexpected: {other:?}"),
    }

    // The streamed half-answer never reached the transcript.
    let msgs = session.snapshot();
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert_eq!(msgs[0].role.as_str(), "user");

    // One announced attempt, discarded without a stream-interrupt reason:
    // a decode failure is not a transport cut.
    let attempts = recorder.events_of(kind::STREAM_ATTEMPT);
    let actions: Vec<&str> = attempts
        .iter()
        .map(|e| e.stream_attempt.as_ref().expect("sa").action.as_str())
        .collect();
    assert_eq!(actions, vec!["begin", "discard"], "{attempts:?}");
    assert_eq!(attempts[1].stream_attempt.as_ref().expect("sa").reason, "");
}

#[tokio::test]
async fn an_idle_cut_stream_is_a_typed_interrupt_and_replays_the_frozen_request() {
    let base = serve_responses(vec![
        premature_close(),
        chunked_sse(&[
            &text_frame("recovered"),
            &usage_frame(8, 4, "stop"),
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

    let out = agent.run("cut").await.expect("turn");
    assert_eq!(out.text, "recovered");

    let attempts = recorder.events_of(kind::STREAM_ATTEMPT);
    let actions: Vec<&str> = attempts
        .iter()
        .map(|e| e.stream_attempt.as_ref().expect("sa").action.as_str())
        .collect();
    assert_eq!(
        actions,
        vec!["begin", "discard", "begin", "commit"],
        "{attempts:?}"
    );
    let discard = attempts[1].stream_attempt.as_ref().expect("sa");
    assert_eq!(discard.reason, "premature_eof", "{attempts:?}");
    let msgs = session.snapshot();
    assert_eq!(msgs.len(), 2, "{msgs:?}");
    assert_eq!(msgs[1].content, "recovered");
}

#[tokio::test]
async fn every_attempt_interrupted_surfaces_the_typed_provider_error() {
    let base = serve_responses(vec![
        premature_close(),
        premature_close(),
        premature_close(),
        premature_close(),
        premature_close(),
        premature_close(),
    ])
    .await;
    let Fixture { agent, session, .. } = fixture(&base, registry_with_readers()).await;

    let err = agent.run("doomed").await.expect_err("interrupted");
    match err {
        reasonix_agent::TurnError::Provider(
            reasonix_provider::ProviderError::StreamInterrupted { reason, .. },
        ) => {
            assert_eq!(reason, "premature_eof");
        }
        other => panic!("unexpected: {other:?}"),
    }
    // Six attempts announced, five discarded, none committed.
    let msgs = session.snapshot();
    assert_eq!(msgs.len(), 1, "{msgs:?}");
}

#[tokio::test]
async fn a_pre_stream_auth_failure_is_terminal_without_replay() {
    let base = serve_responses(vec![
        b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_vec(),
    ])
    .await;
    let Fixture { agent, session, .. } = fixture(&base, registry_with_readers()).await;

    let err = agent.run("locked out").await.expect_err("auth");
    match err {
        reasonix_agent::TurnError::Provider(reasonix_provider::ProviderError::Auth {
            status,
            ..
        }) => {
            assert_eq!(status, 401);
        }
        other => panic!("unexpected: {other:?}"),
    }
    let msgs = session.snapshot();
    assert_eq!(msgs.len(), 1);
}

#[tokio::test]
async fn a_truncated_tool_call_is_dropped_and_named_by_the_host_fact() {
    // finish_reason=length with the last call's arguments cut mid-value:
    // the call reaches neither history nor execution, and the host fact
    // says so (Go: classifyResponseBoundary + recordTruncationFact).
    let dir = tempfile::tempdir().expect("tmp");
    let readable = dir.path().join("ok.txt");
    std::fs::write(&readable, "ok").expect("write");
    let good_args = format!(
        r#"{{"path":"{}"}}"#,
        readable.to_string_lossy().replace('\\', "/")
    );
    let cut_args = r#"{"path":"x","limit":"#.to_owned();
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_1", "read_file", &good_args),
            &tool_call_frame(1, "call_2", "read_file", &cut_args),
            &usage_frame(10, 30, "length"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &tool_call_frame(0, "call_3", "read_file", &good_args),
            &usage_frame(60, 10, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[&text_frame("done."), &usage_frame(70, 5, "stop"), "[DONE]"]),
    ])
    .await;
    let Fixture { agent, session, .. } = fixture(&base, registry_with_readers()).await;

    agent.run("read a lot").await.expect("turn");
    let msgs = session.snapshot();
    // The first assistant turn committed only the complete call.
    assert_eq!(msgs[1].tool_calls.len(), 1, "{:?}", msgs[1].tool_calls);
    assert_eq!(msgs[1].tool_calls[0].id, "call_1");
    // The complete call ran normally and the half-written one never did.
    assert_eq!(msgs[2].role.as_str(), "tool");
    assert_eq!(msgs[2].tool_call_id, "call_1");
    assert!(msgs[2].content.contains("ok"));
    // The host fact rides the round tail, after the tool results (Go: the
    // recordTruncationFact arm of runToolLoop — never between a tool
    // result and its call).
    assert!(msgs[3].host_authored, "{:?}", msgs[3]);
    assert!(
        msgs[3].content.contains("was cut off"),
        "{}",
        msgs[3].content
    );
    assert!(msgs[3].content.contains("read_file"), "{}", msgs[3].content);
    assert!(
        msgs[3].content.contains("did not run"),
        "{}",
        msgs[3].content
    );
}
