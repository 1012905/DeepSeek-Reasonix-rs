//! The `ask` tool and the pending-prompt replay, at the controller boundary
//! (Go: `ask_test.go`, `ask_queue_test.go`, `effect_render_surface_test.go`).
//! A model-issued ask reaches the broker through the call context, its
//! selections come back as the tool result, and a client that reconnects
//! while a question is open re-renders it from the pending snapshot. The
//! tool-level oracles live in `crates/tools/src/builtin/ask.rs`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use reasonix_agent::approval::{ApprovalConfig, AskAnswer, mode};
use reasonix_agent::config::AgentConfig;
use reasonix_agent::sink::Sink;
use reasonix_contract::eventwire::event::Event;
use reasonix_provider::config::{ClientConfig, build_client};
use reasonix_session::turns::{TurnController, TurnControllerParts};
use reasonix_tools::Registry;

fn chunked_sse(frames: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
    );
    for frame in frames {
        let line = format!("data: {frame}\n\n");
        out.extend_from_slice(format!("{:x}\r\n", line.len()).into_bytes().as_slice());
        out.extend_from_slice(line.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"0\r\n\r\n");
    out
}

fn tool_call_frame(index: usize, id: &str, name: &str, arguments: &str) -> String {
    let escaped = arguments.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        r#"{{"choices":[{{"delta":{{"tool_calls":[{{"index":{index},"id":"{id}","function":{{"name":"{name}","arguments":"{escaped}"}}}}]}}}}]}}"#
    )
}

fn text_frame(t: &str) -> String {
    let escaped = t.replace('\\', "\\\\").replace('"', "\\\"");
    format!(r#"{{"choices":[{{"delta":{{"content":"{escaped}"}}}}]}}"#)
}

fn usage_frame(prompt: i64, completion: i64, finish: &str) -> String {
    format!(
        r#"{{"choices":[{{"delta":{{}},"finish_reason":"{finish}"}}],"usage":{{"prompt_tokens":{prompt},"completion_tokens":{completion},"total_tokens":{}}}}}"#,
        prompt + completion
    )
}

async fn serve_responses(responses: Vec<Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for response in responses {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let mut buf = vec![0u8; 65536];
            let _ = sock.read(&mut buf).await;
            sock.write_all(&response).await.ok();
            sock.shutdown().await.ok();
        }
    });
    format!("http://{addr}")
}

#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<Event>>,
}

impl Recorder {
    fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn events_of(&self, name: &str) -> Vec<Event> {
        self.events
            .lock()
            .expect("recorder lock")
            .iter()
            .filter(|e| e.kind == name)
            .cloned()
            .collect()
    }
}

impl Sink for Recorder {
    fn emit(&self, e: Event) {
        self.events.lock().expect("recorder lock").push(e);
    }
}

fn registry() -> Arc<Registry> {
    let registry = Registry::new();
    registry.add(reasonix_tools::builtin::ask_tool());
    Arc::new(registry)
}

fn controller(
    base: &str,
    sink: Arc<dyn Sink>,
    approval: Option<ApprovalConfig>,
) -> Arc<TurnController> {
    let mut cfg = ClientConfig::new("relay", base, "m");
    cfg.api_key = "k".to_owned();
    let client = build_client(&cfg).expect("build client");
    TurnController::new(TurnControllerParts {
        config: AgentConfig {
            model_ref: "relay/m".to_owned(),
            system_prompt: "You are the test kernel.".to_owned(),
            retry_backoff: Some(Duration::ZERO),
            ..AgentConfig::default()
        },
        client,
        registry: registry(),
        sink,
        workspace_root: ".".to_owned(),
        session_dir: String::new(),
        label: "m".to_owned(),
        approval,
    })
}

fn ask_args() -> String {
    r#"{"questions":[{"header":"Library","question":"which library","options":[{"label":"A"},{"label":"B"}]}]}"#
        .to_owned()
}

/// A model-issued `ask` reaches the broker and its selections come back as
/// the tool result (Go `TestAskToolReachesTheUserThroughTheAsker`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_model_issued_ask_is_answered_and_its_selections_reach_the_model() {
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_ask", "ask", &ask_args()),
            &usage_frame(10, 8, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("Picked A."),
            &usage_frame(20, 5, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let rec = Recorder::shared();
    let ctrl = controller(
        &base,
        rec.clone(),
        Some(ApprovalConfig {
            mode: mode::ASK,
            timeout: Duration::from_secs(5),
        }),
    );
    assert!(ctrl.interactive_approval());

    let submit_ctrl = ctrl.clone();
    let submission = tokio::spawn(async move { submit_ctrl.submit("ask me").await });
    let (id, question) = loop {
        let asks = rec.events_of("ask_request");
        if let Some(a) = asks.first().and_then(|e| e.ask.as_ref()) {
            break (a.id.clone(), a.questions[0].clone());
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    assert_eq!(question.id, "q1", "the tool assigns stable question ids");
    assert_eq!(question.header, "Library");

    assert!(ctrl.answer(
        &id,
        vec![AskAnswer {
            id: "q1".to_owned(),
            selected: vec!["A".to_owned()],
        }]
    ));
    let receipt = submission.await.expect("turn task").expect("admitted");
    assert!(receipt.error.is_none(), "the turn completed: {receipt:?}");

    let tool_msg = ctrl
        .session()
        .snapshot()
        .into_iter()
        .find(|m| m.tool_call_id == "call_ask")
        .expect("paired tool message");
    assert_eq!(tool_msg.content, "The user answered:\n- Library: A");

    let notices = rec.events_of("notice");
    assert!(
        notices.iter().any(|e| {
            e.decision_receipt
                .as_ref()
                .is_some_and(|r| r.id == id && r.kind == "ask" && r.outcome == "answered")
        }),
        "the answered receipt reached SSE: {notices:?}"
    );
}

/// The snapshot round-trip: an ask blocks the run, the client "reconnects"
/// (reads the snapshot), and the answer delivered against that snapshot's id
/// lands (Go `TestPendingPromptSnapshotSurvivesReconnect`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pending_ask_appears_in_the_snapshot_and_the_answer_lands() {
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_ask", "ask", &ask_args()),
            &usage_frame(10, 8, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("A it is."),
            &usage_frame(20, 5, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let rec = Recorder::shared();
    let ctrl = controller(
        &base,
        rec.clone(),
        Some(ApprovalConfig {
            mode: mode::ASK,
            timeout: Duration::from_secs(5),
        }),
    );
    assert!(
        !ctrl.pending_prompt(),
        "nothing is pending before a turn starts"
    );

    let submit_ctrl = ctrl.clone();
    let submission = tokio::spawn(async move { submit_ctrl.submit("ask me").await });
    while rec.events_of("ask_request").is_empty() {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    // What a reconnecting frontend reads: the card it must re-render.
    let snapshot = ctrl.pending_prompts();
    assert!(ctrl.pending_prompt());
    assert!(snapshot.approvals.is_empty(), "no tool prompt is open");
    assert_eq!(snapshot.asks.len(), 1, "the question is listed");
    let card = &snapshot.asks[0];
    assert_eq!(card.questions.len(), 1);
    assert_eq!(card.questions[0].header, "Library");
    assert_eq!(card.questions[0].options.len(), 2);

    // The frames a stream re-sends are the same shape the original event had.
    let frames = snapshot.events();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].kind, "ask_request");
    assert_eq!(frames[0].ask.as_ref().expect("ask payload").id, card.id);

    assert!(ctrl.answer(
        &card.id,
        vec![AskAnswer {
            id: "q1".to_owned(),
            selected: vec!["B".to_owned()],
        }]
    ));
    let receipt = submission.await.expect("turn task").expect("admitted");
    assert!(receipt.error.is_none(), "{receipt:?}");
    assert!(
        !ctrl.pending_prompt(),
        "an answered question leaves the snapshot empty"
    );
    assert!(ctrl.pending_prompts().is_empty());

    let tool_msg = ctrl
        .session()
        .snapshot()
        .into_iter()
        .find(|m| m.tool_call_id == "call_ask")
        .expect("paired tool message");
    assert_eq!(tool_msg.content, "The user answered:\n- Library: B");
}

/// An ask blocks the run for its whole timeout, and a snapshot taken while
/// it waits lists the question — never the "no prompt" answer that would
/// make a reconnecting client show a session quietly working.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unanswered_ask_stays_in_the_snapshot_until_the_host_skips_it() {
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_ask", "ask", &ask_args()),
            &usage_frame(10, 8, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[&text_frame("done"), &usage_frame(20, 5, "stop"), "[DONE]"]),
    ])
    .await;
    let rec = Recorder::shared();
    let ctrl = controller(
        &base,
        rec.clone(),
        Some(ApprovalConfig {
            mode: mode::ASK,
            timeout: Duration::from_millis(150),
        }),
    );
    let submit_ctrl = ctrl.clone();
    let submission = tokio::spawn(async move { submit_ctrl.submit("ask me").await });
    while rec.events_of("ask_request").is_empty() {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(ctrl.pending_prompts().asks.len(), 1);
    let receipt = submission.await.expect("turn task").expect("admitted");
    assert!(receipt.error.is_none(), "the turn still completes");
    assert!(
        ctrl.pending_prompts().is_empty(),
        "a skipped question is no longer pending"
    );

    let tool_msg = ctrl
        .session()
        .snapshot()
        .into_iter()
        .find(|m| m.tool_call_id == "call_ask")
        .expect("paired tool message");
    assert!(
        tool_msg.content.starts_with("skipped:"),
        "a timeout is a skip, not a dismissal: {}",
        tool_msg.content
    );
}

/// A headless controller (no approval installed) keeps the Go-nil posture:
/// the ask tool resolves to "unresolved", and the snapshot is empty rather
/// than inventing a card nobody can answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_headless_run_leaves_the_ask_unresolved_and_has_no_snapshot() {
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_ask", "ask", &ask_args()),
            &usage_frame(10, 8, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("no answer"),
            &usage_frame(20, 5, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let rec = Recorder::shared();
    let ctrl = controller(&base, rec.clone(), None);
    assert!(!ctrl.interactive_approval());
    let receipt = ctrl.submit("ask me").await.expect("admitted");
    assert!(receipt.error.is_none(), "{receipt:?}");
    assert!(rec.events_of("ask_request").is_empty(), "nobody was asked");
    assert!(ctrl.pending_prompts().is_empty());

    let tool_msg = ctrl
        .session()
        .snapshot()
        .into_iter()
        .find(|m| m.tool_call_id == "call_ask")
        .expect("paired tool message");
    assert!(
        tool_msg.content.starts_with("unresolved:"),
        "a run with nobody to ask says so: {}",
        tool_msg.content
    );
}
