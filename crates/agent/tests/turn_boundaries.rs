//! Boundary stops: the max-steps grace round, the per-call stop, and the
//! turn cancel (Go: `armFinalizationRound` + `stopUnexecutedBoundaryCalls`
//! + the cancellation paths).

mod support;

use std::sync::Arc;

use reasonix_agent::config::AgentConfig;
use reasonix_agent::sink::kind;
use reasonix_tools::Registry;

use support::{
    Fixture, chunked_sse, fixture, fixture_with_config, registry_with_readers, serve_responses,
    text_frame, tool_call_frame, usage_frame,
};

fn registry_with_writers() -> Arc<Registry> {
    let registry = Registry::new();
    for tool in reasonix_tools::builtin::unconfined_readers() {
        registry.add(tool);
    }
    registry.add(Arc::new(reasonix_tools::builtin::WriteFileTool::default()));
    Arc::new(registry)
}

fn max_steps_cfg(steps: i64) -> AgentConfig {
    AgentConfig {
        max_steps: steps,
        ..AgentConfig::default()
    }
}

/// write_file arguments for one tempdir target: a relative path would land
/// next to the test binary, and the tests must leave nothing behind.
fn write_args_for(path: &std::path::Path) -> String {
    format!(
        r#"{{"path":"{}","content":"w"}}"#,
        path.to_string_lossy().replace('\\', "/")
    )
}

fn read_args_for(path: &std::path::Path) -> String {
    format!(
        r#"{{"path":"{}"}}"#,
        path.to_string_lossy().replace('\\', "/")
    )
}

#[tokio::test]
async fn max_steps_arms_a_grace_round_that_finalizes_cleanly() {
    let target = tempfile::tempdir().expect("tmp").path().join("out.txt");
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_1", "write_file", &write_args_for(&target)),
            &usage_frame(10, 8, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("final summary"),
            &usage_frame(30, 5, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let Fixture {
        agent,
        recorder,
        session,
        ..
    } = fixture_with_config(&base, registry_with_writers(), max_steps_cfg(1)).await;

    // The grace round's answer is a clean final, not a pause.
    let out = agent.run("do one thing").await.expect("turn");
    assert_eq!(out.text, "final summary");
    assert_eq!(out.stop, reasonix_agent::StopCause::Final);

    // The nudge landed between the tool result and the grace request.
    let msgs = session.snapshot();
    assert!(
        msgs.iter()
            .any(|m| m.host_authored && m.content.contains("tool-call round limit")),
        "{msgs:?}"
    );
    let notices = recorder.events_of(kind::NOTICE);
    assert!(
        notices.iter().any(|n| n.code == "tool_budget"),
        "{notices:?}"
    );
}

#[tokio::test]
async fn the_grace_round_refuses_late_calls_and_pauses() {
    let target = tempfile::tempdir().expect("tmp").path().join("out.txt");
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_1", "write_file", &write_args_for(&target)),
            &usage_frame(10, 8, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &tool_call_frame(0, "call_2", "read_file", &read_args_for(&target)),
            &usage_frame(40, 8, "tool_calls"),
            "[DONE]",
        ]),
    ])
    .await;
    let Fixture { agent, session, .. } =
        fixture_with_config(&base, registry_with_writers(), max_steps_cfg(1)).await;

    let out = match agent.run("do one thing").await {
        Ok(out) => out,
        Err(err) => panic!("run failed: {err:?}"),
    };
    assert_eq!(out.text, "");
    assert_eq!(
        out.stop,
        reasonix_agent::StopCause::MaxSteps {
            steps: 1,
            key: "max_steps".to_owned()
        }
    );

    // The late call is paired with the grace refusal, never executed.
    let msgs = session.snapshot();
    let late = &msgs[msgs.len() - 1];
    assert_eq!(late.role.as_str(), "tool");
    assert_eq!(late.tool_call_id, "call_2");
    assert!(
        late.content.contains("no more tools will run in this turn"),
        "{}",
        late.content
    );
}

#[tokio::test]
async fn cancelling_the_turn_ends_it_and_keeps_completed_tool_pairs() {
    use reasonix_agent::seams::Gate;
    struct CancelOnFirstCall {
        agent: Arc<reasonix_agent::Agent>,
        fired: std::sync::atomic::AtomicBool,
    }
    impl Gate for CancelOnFirstCall {
        fn check(
            &self,
            _tool_name: &str,
            _args: &serde_json::Value,
            _read_only: bool,
        ) -> Result<bool, String> {
            // Allow the call and cancel the turn: the loop finds the token
            // set after the batch and ends it, keeping the completed pair
            // (Go: the ctx.Err() arm after the batch).
            self.fired.swap(true, std::sync::atomic::Ordering::SeqCst);
            self.agent.cancel();
            Ok(true)
        }
    }

    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_1", "read_file", r#"{"path":"x.txt"}"#),
            &usage_frame(10, 4, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("never reached"),
            &usage_frame(30, 4, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let Fixture { agent, session, .. } = fixture(&base, registry_with_readers()).await;
    agent.set_gate(Arc::new(CancelOnFirstCall {
        agent: agent.clone(),
        fired: std::sync::atomic::AtomicBool::new(false),
    }));

    let err = agent.run("cancel me").await.expect_err("cancelled");
    assert!(
        matches!(err, reasonix_agent::TurnError::Cancelled),
        "{err:?}"
    );
    // The completed tool pair stays in the transcript.
    let msgs = session.snapshot();
    assert!(msgs.len() >= 3, "{msgs:?}");
    assert_eq!(msgs[2].role.as_str(), "tool");
    assert_eq!(msgs[2].tool_call_id, "call_1");
}

#[tokio::test]
async fn a_cancelled_token_ends_every_later_turn_before_sampling() {
    let base = serve_responses(vec![chunked_sse(&[&text_frame("ok"), "[DONE]"])]).await;
    let Fixture { agent, .. } = fixture(&base, registry_with_readers()).await;
    agent.cancel();
    let err = agent.run("q").await.expect_err("cancelled");
    assert!(
        matches!(err, reasonix_agent::TurnError::Cancelled),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_stopped_call_runs_nothing_and_is_paired_with_the_cancelled_output() {
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_1", "read_file", r#"{"path":"x.txt"}"#),
            &usage_frame(10, 4, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("moving on"),
            &usage_frame(20, 4, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let Fixture { agent, session, .. } = fixture(&base, registry_with_readers()).await;
    agent.cancel_tool_call("call_1");

    agent.run("skip that call").await.expect("turn");
    let msgs = session.snapshot();
    let tool_msg = &msgs[2];
    assert_eq!(tool_msg.tool_call_id, "call_1");
    assert_eq!(
        tool_msg.content,
        "cancelled: context cancelled before execution"
    );
    assert!(!tool_msg.tool_failure.as_ref().expect("f").blocked);
}

#[tokio::test]
async fn the_permission_gate_blocks_one_call_and_the_turn_recovers() {
    use reasonix_agent::seams::Gate;
    struct DenyWrites;
    impl Gate for DenyWrites {
        fn check(
            &self,
            tool_name: &str,
            _args: &serde_json::Value,
            read_only: bool,
        ) -> Result<bool, String> {
            if tool_name == "write_file" || !read_only {
                Ok(false)
            } else {
                Ok(true)
            }
        }
    }

    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(
                0,
                "call_1",
                "write_file",
                &write_args_for(&tempfile::tempdir().expect("tmp").path().join("out.txt")),
            ),
            &usage_frame(10, 8, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("I asked; you declined."),
            &usage_frame(20, 5, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let Fixture { agent, session, .. } = fixture(&base, registry_with_writers()).await;
    agent.set_gate(Arc::new(DenyWrites));

    let out = agent.run("write it").await.expect("turn");
    assert_eq!(out.text, "I asked; you declined.");
    let msgs = session.snapshot();
    let tool_msg = &msgs[2];
    assert!(tool_msg.content.contains("blocked"), "{}", tool_msg.content);
    assert!(tool_msg.tool_failure.as_ref().expect("f").blocked);
}

#[tokio::test]
async fn a_failed_writer_opens_the_batch_barrier() {
    // A writer that fails (missing path) stops later mutations in the same
    // batch; a trailing read still runs (Go: the mutationDependencyBarrier).
    let dir = tempfile::tempdir().expect("tmp");
    let readable = dir.path().join("a.txt");
    std::fs::write(&readable, "alpha").expect("write");
    let failed = tool_call_frame(0, "call_w", "write_file", r#"{"path":"","content":"x"}"#);
    let later_writer = tool_call_frame(
        1,
        "call_v",
        "write_file",
        r#"{"path":"b.txt","content":"y"}"#,
    );
    let reader_args = format!(
        r#"{{"path":"{}"}}"#,
        readable.to_string_lossy().replace('\\', "/")
    );
    let reader = tool_call_frame(2, "call_r", "read_file", &reader_args);
    let base = serve_responses(vec![
        chunked_sse(&[
            &failed,
            &later_writer,
            &reader,
            &usage_frame(10, 9, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("handled."),
            &usage_frame(40, 4, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let Fixture { agent, session, .. } = fixture(&base, registry_with_writers()).await;

    agent.run("batch").await.expect("turn");
    let msgs = session.snapshot();
    let tool_msgs: Vec<_> = msgs.iter().filter(|m| m.role.as_str() == "tool").collect();
    assert_eq!(tool_msgs.len(), 3, "{msgs:?}");
    assert!(
        tool_msgs[0].content.contains("error:") || tool_msgs[0].tool_failure.is_some(),
        "{}",
        tool_msgs[0].content
    );
    assert!(
        tool_msgs[1]
            .content
            .contains("skipped because an earlier modification"),
        "{}",
        tool_msgs[1].content
    );
    assert_eq!(
        tool_msgs[1].tool_failure.as_ref().expect("f").refusal_code,
        "batch.skipped_after_failed_change"
    );
    // The read-only diagnosis still ran.
    assert!(
        tool_msgs[2].tool_failure.is_none(),
        "{}",
        tool_msgs[2].content
    );
}

#[tokio::test]
async fn the_steer_seam_lands_guidance_before_the_next_request() {
    struct OneSteer(std::sync::Mutex<Option<String>>);
    impl reasonix_agent::seams::SteerInbox for OneSteer {
        fn drain(&self) -> Option<String> {
            self.0.lock().expect("steer").take()
        }
    }
    let base = serve_responses(vec![
        chunked_sse(&[
            &text_frame("round one"),
            &usage_frame(5, 2, "stop"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("round two"),
            &usage_frame(10, 2, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let Fixture { agent, session, .. } = fixture(&base, registry_with_readers()).await;
    agent.set_steer_inbox(Arc::new(OneSteer(std::sync::Mutex::new(Some(
        "prefer rust".to_owned(),
    )))));

    // First run drains the queued guidance before its request.
    agent.run("start").await.expect("turn");
    let msgs = session.snapshot();
    assert!(
        msgs.iter()
            .any(|m| m.host_authored && m.content == "prefer rust"),
        "{msgs:?}"
    );
}
