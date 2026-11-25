//! The controller write side at the provider boundary: submit runs a turn
//! and lands its output on the user's sink, the admission save makes the
//! prompt durable before the answer, cancel ends the in-flight turn, and
//! steer lands mid-turn guidance (Go: control's turn-group tests, rebuilt
//! over the canned-provider pattern the agent tests use).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use reasonix_agent::config::AgentConfig;
use reasonix_agent::sink::{Sink, kind};
use reasonix_contract::eventwire::event::Event;
use reasonix_provider::config::{ClientConfig, build_client};
use reasonix_session::controller::Controller;
use reasonix_session::turns::{SubmitRefusal, TurnController, TurnControllerParts};
use reasonix_tools::Registry;

/// HTTP/1.1 chunked SSE response around the given data frames.
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

/// One-shot HTTP server answering each accepted connection with the next
/// canned response, then closing.
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

/// A server whose response headers arrive so the stream starts, but whose
/// body never does: the turn stays in flight until the caller cancels.
async fn serve_hanging_stream() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        let mut buf = vec![0u8; 65536];
        let _ = sock.read(&mut buf).await;
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n")
            .await
            .ok();
        // The body never arrives; the socket stays open.
        tokio::time::sleep(Duration::from_secs(3600)).await;
    });
    format!("http://{addr}")
}

/// Collects every event the controller and its agent emit, in emit order.
#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<Event>>,
}

impl Recorder {
    fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn snapshot(&self) -> Vec<Event> {
        self.events.lock().expect("recorder lock").clone()
    }

    fn events_of(&self, name: &str) -> Vec<Event> {
        self.snapshot()
            .into_iter()
            .filter(|e| e.kind == name)
            .collect()
    }
}

impl Sink for Recorder {
    fn emit(&self, e: Event) {
        self.events.lock().expect("recorder lock").push(e);
    }
}

#[derive(Default)]
struct RunningAtDone {
    controller: Mutex<Option<Arc<TurnController>>>,
    running: Mutex<Option<bool>>,
}

impl Sink for RunningAtDone {
    fn emit(&self, e: Event) {
        if e.kind == reasonix_session::turns::TURN_DONE {
            let controller = self.controller.lock().expect("controller lock");
            let running = controller.as_ref().expect("bound controller").running();
            *self.running.lock().expect("running lock") = Some(running);
        }
    }
}

/// One controller over the canned endpoint with the standard fixtures.
async fn controller(base: &str, sink: Arc<dyn Sink>) -> Arc<TurnController> {
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
        registry: Arc::new(Registry::new()),
        sink,
        workspace_root: "C:\\work\\proj".to_owned(),
        session_dir: String::new(),
        label: "m".to_owned(),
        approval: None,
    })
}

#[tokio::test]
async fn submit_runs_a_turn_and_lands_the_answer_on_the_sink() {
    let dir = tempfile::tempdir().expect("tmp");
    let recorder = Recorder::shared();
    let base = serve_responses(vec![chunked_sse(&[
        &text_frame("the final answer"),
        &usage_frame(7, 3, "stop"),
        "[DONE]",
    ])])
    .await;
    let ctrl = controller(&base, recorder.clone()).await;
    ctrl.set_session_dir(&dir.path().to_string_lossy());

    let receipt = ctrl.submit("hello").await.expect("turn admitted");
    assert!(receipt.error.is_none(), "{:?}", receipt.error);
    assert!(!receipt.cancelled);

    // The transcript carries the exchange, opened by the cache-stable prompt.
    let msgs = ctrl.session().snapshot();
    assert_eq!(msgs[0].role.as_str(), "system");
    assert!(
        msgs.iter()
            .any(|m| m.role.as_str() == "user" && m.content == "hello")
    );
    assert!(
        msgs.iter()
            .any(|m| m.role.as_str() == "assistant" && m.content.contains("the final answer"))
    );

    // The user's sink saw the turn's output and exactly one completion.
    let done = recorder.events_of(reasonix_session::turns::TURN_DONE);
    assert_eq!(done.len(), 1, "{:?}", recorder.snapshot());
    assert!(done[0].err.is_empty());
    assert!(!done[0].cancelled);
    assert!(recorder.snapshot().iter().any(|e| e.kind == kind::TEXT));

    // The turn-end snapshot put the whole exchange on disk.
    let bytes = std::fs::read(ctrl.session_path()).expect("transcript bytes");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("hello"), "{text}");
    assert!(text.contains("the final answer"), "{text}");
}

#[tokio::test]
async fn admission_stays_closed_until_the_completion_event_is_emitted() {
    let sink = Arc::new(RunningAtDone::default());
    let base = serve_responses(vec![chunked_sse(&[
        &text_frame("done"),
        &usage_frame(1, 1, "stop"),
        "[DONE]",
    ])])
    .await;
    let ctrl = controller(&base, sink.clone()).await;
    *sink.controller.lock().expect("controller lock") = Some(ctrl.clone());

    ctrl.submit("hello").await.expect("turn admitted");

    assert_eq!(*sink.running.lock().expect("running lock"), Some(true));
    assert!(!ctrl.running());
}

#[tokio::test]
async fn the_admitted_prompt_is_durable_before_the_answer() {
    let dir = tempfile::tempdir().expect("tmp");
    let recorder = Recorder::shared();
    // The endpoint never answers usefully: the turn fails, but the prompt
    // is already durable (Go: a process killed mid-turn still has it).
    let base = serve_responses(vec![
        b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_vec(),
    ])
    .await;
    let ctrl = controller(&base, recorder.clone()).await;
    ctrl.set_session_dir(&dir.path().to_string_lossy());

    let receipt = ctrl
        .submit("the durable prompt")
        .await
        .expect("turn admitted");
    let err = receipt.error.expect("provider failure");
    assert!(
        matches!(err, reasonix_agent::TurnError::Provider(_)),
        "{err:?}"
    );

    let bytes = std::fs::read(ctrl.session_path()).expect("transcript bytes");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("the durable prompt"), "{text}");
    assert!(!text.contains("assistant"), "{text}");

    let done = recorder.events_of(reasonix_session::turns::TURN_DONE);
    assert_eq!(done.len(), 1);
    assert!(!done[0].err.is_empty(), "the failure reaches the wire");
}

#[tokio::test]
async fn a_running_turn_refuses_a_second_submit_and_cancel_ends_the_first() {
    let recorder = Recorder::shared();
    let base = serve_hanging_stream().await;
    let ctrl = controller(&base, recorder.clone()).await;

    let first = tokio::spawn({
        let ctrl = ctrl.clone();
        async move { ctrl.submit("in flight").await }
    });
    // The turn is admitted once the agent machine reports it running.
    while !ctrl.running() {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(matches!(
        ctrl.submit("while busy").await,
        Err(SubmitRefusal::Busy)
    ));

    ctrl.cancel();
    let receipt = first.await.expect("join").expect("turn admitted");
    assert!(receipt.cancelled, "{receipt:?}");
    // The spine and the provider transport each carry the abort; either
    // spelling is the cancelled terminal.
    assert!(
        matches!(
            receipt.error,
            Some(reasonix_agent::TurnError::Cancelled)
                | Some(reasonix_agent::TurnError::Provider(
                    reasonix_provider::ProviderError::Cancelled
                ))
        ),
        "{receipt:?}"
    );
    assert!(!ctrl.running());

    let done = recorder.events_of(reasonix_session::turns::TURN_DONE);
    assert_eq!(done.len(), 1);
    assert!(done[0].cancelled, "{:?}", done[0]);

    // A refused submission started nothing: the transcript holds only the
    // cancelled turn's prompt.
    let msgs = ctrl.session().snapshot();
    let user_turns: Vec<_> = msgs.iter().filter(|m| m.role.as_str() == "user").collect();
    assert_eq!(user_turns.len(), 1, "{msgs:?}");
}

#[tokio::test]
async fn steer_lands_mid_turn_guidance_at_the_next_round_boundary() {
    let recorder = Recorder::shared();
    let base = serve_responses(vec![
        chunked_sse(&[
            &text_frame("done one"),
            &usage_frame(5, 1, "stop"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("done two"),
            &usage_frame(5, 1, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let ctrl = controller(&base, recorder.clone()).await;

    ctrl.submit("first").await.expect("turn one");
    ctrl.steer("focus on the config file");
    ctrl.submit("second").await.expect("turn two");

    let msgs = ctrl.session().snapshot();
    let steered = msgs
        .iter()
        .find(|m| m.host_authored && m.content.contains("focus on the config file"));
    assert!(steered.is_some(), "{msgs:?}");
}

#[tokio::test]
async fn the_frozen_read_surface_tracks_the_pinned_paths() {
    let dir = tempfile::tempdir().expect("tmp");
    let ctrl = controller("http://127.0.0.1:1", Recorder::shared()).await;
    assert_eq!(ctrl.workspace_root(), "C:\\work\\proj");
    assert!(ctrl.session_path().is_empty());

    ctrl.set_session_dir(&dir.path().to_string_lossy());
    ctrl.ensure_session_path();
    let first = ctrl.session_path();
    assert!(!first.is_empty());
    assert!(first.ends_with(".jsonl"), "{first}");
    // Minting is pinned-once: a second ensure keeps the same file.
    ctrl.ensure_session_path();
    assert_eq!(ctrl.session_path(), first);

    // An explicit pin wins over the minted path.
    let pinned = dir.path().join("pinned.jsonl");
    ctrl.set_session_path(&pinned.to_string_lossy());
    assert_eq!(ctrl.session_path(), pinned.to_string_lossy());
    // A snapshot without a pinned path is a no-op; with one it writes.
    ctrl.snapshot().expect("snapshot");
    assert!(pinned.exists());
    // The controller implements the frozen hub surface.
    let read: Arc<dyn reasonix_session::controller::Controller> = ctrl.clone();
    assert_eq!(read.session_path(), pinned.to_string_lossy());
}
