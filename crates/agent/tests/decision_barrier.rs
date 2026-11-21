//! The decision barrier at the turn boundary (Go: `execute_batch.go`'s
//! `scheduleUpToDecision` + `markDeferredAfterDecision`). A reply that asks
//! the user a question stops there: every call written beside it was written
//! before the answer existed, so none of them runs and each is paired with
//! the reason.

mod support;

use std::sync::Arc;
use std::time::Duration;

use reasonix_agent::approval::{ApprovalBroker, ApprovalConfig, AskAnswer, BrokerAsker, mode};
use reasonix_tools::Registry;

use support::{
    Fixture, chunked_sse, fixture, serve_responses, text_frame, tool_call_frame, usage_frame,
};

fn ask_args() -> String {
    r#"{"questions":[{"header":"Library","question":"which","options":[{"label":"A"},{"label":"B"}]}]}"#
        .to_owned()
}

fn write_args_for(path: &std::path::Path) -> String {
    format!(
        r#"{{"path":"{}","content":"beside the question"}}"#,
        path.to_string_lossy().replace('\\', "/")
    )
}

fn barrier_registry() -> Arc<Registry> {
    let registry = Registry::new();
    for tool in reasonix_tools::builtin::unconfined_readers() {
        registry.add(tool);
    }
    registry.add(Arc::new(reasonix_tools::builtin::WriteFileTool::default()));
    registry.add(reasonix_tools::builtin::ask_tool());
    Arc::new(registry)
}

/// The writer a reply put after its question does not run: it was authored
/// before the answer, and the round ends at the ask (Go
/// `markDeferredAfterDecision`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_written_after_a_question_is_not_carried_over() {
    let dir = tempfile::tempdir().expect("tmp");
    let target = dir.path().join("beside.txt");
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_ask", "ask", &ask_args()),
            &tool_call_frame(1, "call_write", "write_file", &write_args_for(&target)),
            &usage_frame(10, 8, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("now I know"),
            &usage_frame(20, 5, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let Fixture {
        agent,
        recorder,
        session,
    } = fixture(&base, barrier_registry()).await;

    let broker = ApprovalBroker::new(
        ApprovalConfig {
            mode: mode::ASK,
            timeout: Duration::from_secs(5),
        },
        session.clone(),
        recorder.clone(),
    );
    agent.set_asker(Arc::new(BrokerAsker(broker.clone())));

    let run_agent = agent.clone();
    let run = tokio::spawn(async move { run_agent.run("ask and write").await });
    let id = loop {
        let asks = recorder.events_of("ask_request");
        if let Some(a) = asks.first().and_then(|e| e.ask.as_ref()) {
            break a.id.clone();
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    assert!(!target.exists(), "nothing beside the question ran yet");
    assert!(broker.answer(
        &id,
        vec![AskAnswer {
            id: "q1".to_owned(),
            selected: vec!["A".to_owned()],
        }]
    ));

    let out = run.await.expect("run task").expect("the turn completes");
    assert_eq!(out.text, "now I know");
    assert!(
        !target.exists(),
        "the writer written beside the question never ran"
    );

    let msgs = session.snapshot();
    let ask_msg = msgs
        .iter()
        .find(|m| m.tool_call_id == "call_ask")
        .expect("the ask is paired");
    assert_eq!(
        ask_msg.content, "The user answered:\n- Library: A",
        "the answer is the ask's result"
    );
    let write_msg = msgs
        .iter()
        .find(|m| m.tool_call_id == "call_write")
        .expect("the deferred writer is paired");
    assert!(
        write_msg
            .content
            .starts_with("not run: this round stopped at a question"),
        "the deferred call says why: {}",
        write_msg.content
    );
    assert!(
        write_msg.tool_failure.as_ref().is_some_and(|f| f.blocked),
        "a deferred call is blocked, not a plain answer"
    );

    // Both result cards reached the frontend, in call order.
    let ids: Vec<String> = recorder
        .events_of("tool_result")
        .iter()
        .filter_map(|e| e.tool.as_ref().map(|t| t.id.clone()))
        .collect();
    assert_eq!(ids, vec!["call_ask".to_owned(), "call_write".to_owned()]);
}

/// An ask on its own ends nothing early: the round's only call runs and the
/// loop carries on with the answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lone_ask_runs_and_the_turn_continues() {
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_ask", "ask", &ask_args()),
            &usage_frame(10, 8, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[&text_frame("thanks"), &usage_frame(20, 5, "stop"), "[DONE]"]),
    ])
    .await;
    let Fixture {
        agent,
        recorder,
        session,
    } = fixture(&base, barrier_registry()).await;
    let broker = ApprovalBroker::new(
        ApprovalConfig {
            mode: mode::ASK,
            timeout: Duration::from_secs(5),
        },
        session.clone(),
        recorder.clone(),
    );
    agent.set_asker(Arc::new(BrokerAsker(broker.clone())));

    let run_agent = agent.clone();
    let run = tokio::spawn(async move { run_agent.run("just ask").await });
    let id = loop {
        let asks = recorder.events_of("ask_request");
        if let Some(a) = asks.first().and_then(|e| e.ask.as_ref()) {
            break a.id.clone();
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    assert!(broker.answer(
        &id,
        vec![AskAnswer {
            id: "q1".to_owned(),
            selected: vec!["B".to_owned()],
        }]
    ));
    let out = run.await.expect("run task").expect("the turn completes");
    assert_eq!(out.text, "thanks");
    let msg = session
        .snapshot()
        .into_iter()
        .find(|m| m.tool_call_id == "call_ask")
        .expect("paired");
    assert_eq!(msg.content, "The user answered:\n- Library: B");
}
