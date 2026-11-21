//! The approval gate at the controller boundary (Go:
//! `approval_e2e_test.go`, `headless_approval_test.go`,
//! `effect_ask_timeout_test.go`): a tool call is gated → asked → answered →
//! executed, and the unanswered path is refused with a typed identity.
//! The seam-level oracles live in `crates/agent/tests/approval.rs`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use reasonix_agent::approval::{ApprovalConfig, DecisionInput, mode};
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

fn write_registry() -> Arc<Registry> {
    let registry = Registry::new();
    registry.add(Arc::new(reasonix_tools::builtin::WriteFileTool::default()));
    Arc::new(registry)
}

fn write_args(path: &std::path::Path) -> String {
    format!(
        r#"{{"path":"{}","content":"approved"}}"#,
        path.to_string_lossy().replace('\\', "/")
    )
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
        registry: write_registry(),
        sink,
        workspace_root: ".".to_owned(),
        session_dir: String::new(),
        label: "m".to_owned(),
        approval,
    })
}

/// Go `TestApprovalToolWideEndToEnd`: the model calls a writer, the call is
/// gated, a human approves it, and only then does the file appear.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gated_tool_call_is_asked_answered_then_executed() {
    let dir = tempfile::tempdir().expect("tmp");
    let target = dir.path().join("approved.txt");
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_1", "write_file", &write_args(&target)),
            &usage_frame(10, 8, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[&text_frame("Done."), &usage_frame(20, 5, "stop"), "[DONE]"]),
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
    let submission = tokio::spawn(async move { submit_ctrl.submit("write it").await });
    // Wait for the gate's approval_request, then answer with the id it issued.
    let id = loop {
        let reqs = rec.events_of("approval_request");
        if let Some(a) = reqs.first().and_then(|e| e.approval.as_ref()) {
            break a.id.clone();
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    assert!(!target.exists(), "nothing ran before the human answered");
    assert!(ctrl.approve(
        &id,
        DecisionInput {
            allow: true,
            session: false,
            persist: false
        }
    ));
    let receipt = submission.await.expect("turn task").expect("admitted");
    assert!(receipt.error.is_none(), "the turn completed");
    assert!(target.exists(), "the approved write ran");
    assert_eq!(std::fs::read_to_string(&target).expect("file"), "approved");

    let notices = rec.events_of("notice");
    assert!(
        notices.iter().any(|e| {
            e.decision_receipt
                .as_ref()
                .is_some_and(|r| r.id == id && r.kind == "tool" && r.outcome == "allow_once")
        }),
        "the decision receipt reached SSE: {notices:?}"
    );
    let tool_results = rec.events_of("tool_result");
    assert!(
        tool_results
            .iter()
            .all(|e| { e.tool.as_ref().is_none_or(|t| t.refusal_code.is_empty()) }),
        "no refusal on an approved call"
    );
}

/// Go `TestApprovalTimeoutDeniesWhenUnanswered` at the turn boundary: a
/// walked-away user cannot wedge the loop, and the call carries a typed
/// refusal into the transcript instead of running.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unanswered_gate_expires_into_a_typed_refusal() {
    let dir = tempfile::tempdir().expect("tmp");
    let target = dir.path().join("blocked.txt");
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_1", "write_file", &write_args(&target)),
            &usage_frame(10, 8, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("It was blocked."),
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
            timeout: Duration::from_millis(60),
        }),
    );
    let start = std::time::Instant::now();
    let receipt = ctrl.submit("write it").await.expect("admitted");
    assert!(receipt.error.is_none(), "the turn still completes");
    assert!(start.elapsed() < Duration::from_secs(2), "near the timeout");
    assert!(!target.exists(), "no approval means no write");

    let msgs = ctrl.session().snapshot();
    let tool_msg = msgs
        .iter()
        .find(|m| m.tool_call_id == "call_1")
        .expect("paired tool message");
    let failure = tool_msg.tool_failure.as_ref().expect("tool failure");
    assert!(
        failure.blocked,
        "the call was blocked, not declined by the tool"
    );
    assert_eq!(
        failure.refusal_code,
        reasonix_safety::permission::REFUSAL_UNATTENDED,
        "the refusal carries a typed identity: {failure:?}"
    );

    let reqs = rec.events_of("approval_request");
    assert_eq!(reqs.len(), 1, "the prompt reached the frontend");
    let id = reqs[0].approval.as_ref().unwrap().id.clone();
    let notices = rec.events_of("notice");
    assert!(
        notices.iter().any(|e| {
            e.decision_receipt
                .as_ref()
                .is_some_and(|r| r.id == id && r.kind == "tool" && r.outcome == "deny")
        }),
        "a timed-out prompt is recorded as a host denial: {notices:?}"
    );
}
