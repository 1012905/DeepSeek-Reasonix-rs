//! The text-only turn: one sampling round, deltas to the sink, the commit
//! boundary, and the final answer (Go: runToolLoop's no-calls arm).

mod support;

use reasonix_agent::sink::kind;

use support::{
    Fixture, chunked_sse, fixture, registry_with_readers, serve_responses, text_frame, usage_frame,
};

#[tokio::test]
async fn a_text_turn_streams_deltas_and_lands_the_answer() {
    let base = serve_responses(vec![chunked_sse(&[
        &text_frame("hel"),
        &text_frame("lo world"),
        &usage_frame(12, 3, "stop"),
        "[DONE]",
    ])])
    .await;
    let Fixture {
        agent,
        recorder,
        session,
        ..
    } = fixture(&base, registry_with_readers()).await;

    let out = agent.run("say hello").await.expect("turn");

    assert_eq!(out.text, "hello world");
    assert_eq!(out.stop, reasonix_agent::StopCause::Final);
    assert_eq!(out.usage.as_ref().expect("usage").prompt_tokens, 12);
    assert!(out.work_duration_ms >= 1);

    // The transcript holds exactly the user turn and the committed answer.
    let msgs = session.snapshot();
    assert_eq!(msgs.len(), 2, "{msgs:?}");
    assert_eq!(msgs[0].role.as_str(), "user");
    assert_eq!(msgs[0].content, "say hello");
    assert!(msgs[0].created_at > 0);
    assert_eq!(msgs[1].role.as_str(), "assistant");
    assert_eq!(msgs[1].content, "hello world");
    assert_eq!(msgs[1].model_ref, "relay/m");
    assert!(msgs[1].work_duration_ms >= 1);

    // Event order: the turn opens, deltas stream live, usage closes it.
    let kinds = recorder.kinds();
    let started = kinds
        .iter()
        .position(|k| k == kind::TURN_STARTED)
        .expect("started");
    let first_text = kinds.iter().position(|k| k == kind::TEXT).expect("text");
    let usage = kinds.iter().position(|k| k == kind::USAGE).expect("usage");
    assert!(started < first_text && first_text < usage, "{kinds:?}");
    assert_eq!(recorder.text(), "hello world");
    // The turn phase marker and the named message index ride the opening.
    let opened = recorder.events_of(kind::TURN_STARTED);
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].msg_index, Some(0));
    assert_eq!(opened[0].model_ref, "relay/m");
    assert!(
        recorder
            .events_of(kind::TURN_PHASE)
            .iter()
            .any(|e| e.phase == "working")
    );
}

#[tokio::test]
async fn an_empty_final_is_retried_then_fails_the_turn() {
    // Two rounds of nothing visible, then a visible answer: the retry
    // nudge lands as a host-authored message (Go: maxEmptyFinalBlocks).
    let base = serve_responses(vec![
        chunked_sse(&[&usage_frame(5, 0, "stop"), "[DONE]"]),
        chunked_sse(&[
            &text_frame("now I answer"),
            &usage_frame(9, 3, "stop"),
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

    let out = agent.run("answer me").await.expect("turn");
    assert_eq!(out.text, "now I answer");

    let msgs = session.snapshot();
    assert_eq!(msgs.len(), 4, "{msgs:?}");
    assert_eq!(msgs[2].role.as_str(), "user");
    assert!(msgs[2].host_authored, "{:?}", msgs[2]);
    assert!(
        msgs[2].content.contains("visible answer"),
        "{}",
        msgs[2].content
    );
    let notices = recorder.events_of(kind::NOTICE);
    assert!(
        notices
            .iter()
            .any(|n| n.code == "empty_final" && n.level == "info"),
        "{notices:?}"
    );
}

#[tokio::test]
async fn three_empty_finals_fail_the_turn() {
    let base = serve_responses(vec![
        chunked_sse(&[&usage_frame(5, 0, "stop"), "[DONE]"]),
        chunked_sse(&[&usage_frame(6, 0, "stop"), "[DONE]"]),
        chunked_sse(&[&usage_frame(7, 0, "stop"), "[DONE]"]),
    ])
    .await;
    let Fixture { agent, session, .. } = fixture(&base, registry_with_readers()).await;

    let err = agent.run("answer me").await.expect_err("empty final");
    match err {
        reasonix_agent::TurnError::EmptyFinal {
            blocks,
            finish_reason,
        } => {
            assert_eq!(blocks, 3);
            assert_eq!(finish_reason, "stop");
        }
        other => panic!("unexpected: {other:?}"),
    }
    // Two retry nudges land (the failing round gets none — Go adds no
    // nudge when the budget is exhausted); every round still committed its
    // assistant message, because the commit boundary is the clean terminal,
    // not the answer's usefulness.
    let msgs = session.snapshot();
    assert_eq!(msgs.len(), 6, "{msgs:?}"); // user + 3 assistant + 2 nudges
    assert_eq!(msgs.iter().filter(|m| m.role.as_str() == "user").count(), 3);
}
