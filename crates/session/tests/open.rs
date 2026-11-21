//! The hub open/publish machine at the provider boundary (Go serve
//! `hub.go`'s Open tests, rebuilt over the canned-provider pattern the
//! turn tests use): open builds a runtime whose submit runs end-to-end
//! through the hub, a driven session refuses a second open with the
//! holder, and the frozen read surface plus broadcaster report what was
//! published.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use reasonix_agent::sink::{Sink, kind};
use reasonix_contract::eventwire::event::Event;
use reasonix_session::controller::{BroadcasterHandle, Controller};
use reasonix_session::hub::{
    Hub, HubOptions, HubPaths, RuntimeKit, RuntimeKitFactory, RuntimeSpec,
};
use reasonix_session::model::{HubRefusal, OpenRuntimeRequest, RuntimeId};

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

fn tool_call_frame(index: usize, id: &str, name: &str, arguments: &str) -> String {
    let escaped = arguments.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        r#"{{"choices":[{{"delta":{{"tool_calls":[{{"index":{index},"id":"{id}","function":{{"name":"{name}","arguments":"{escaped}"}}}}]}}}}]}}"#
    )
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

/// Collects every event the opened runtime's turns emit, in emit order.
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

    fn snapshot(&self) -> Vec<Event> {
        self.events.lock().expect("recorder lock").clone()
    }
}

impl Sink for Recorder {
    fn emit(&self, e: Event) {
        self.events.lock().expect("recorder lock").push(e);
    }
}

/// A fan-out handle the test can compare by pointer (Go test doubles of
/// `*Broadcaster`).
struct TestBroadcaster;

impl BroadcasterHandle for TestBroadcaster {
    fn subscribe(
        &self,
        _mode: reasonix_session::fanout::SubscriptionMode,
    ) -> reasonix_session::fanout::Subscription {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        reasonix_session::fanout::Subscription::new(Box::new(rx), Box::new(|| {}))
    }
    fn watermark(&self) -> i64 {
        0
    }
    fn replay(&self, _after: i64) -> (Vec<bytes::Bytes>, bool) {
        (Vec::new(), true)
    }
}

/// The test's pane composition: one broadcaster handle and the recorder
/// sink per open, both kept so the test can prove the hub adopted them.
struct PaneKit {
    sink: Arc<Recorder>,
    kits: Mutex<Vec<RuntimeKit>>,
    approval: Option<reasonix_agent::approval::ApprovalConfig>,
}

impl PaneKit {
    fn shared() -> Arc<Self> {
        Arc::new(Self {
            sink: Recorder::shared(),
            kits: Mutex::new(Vec::new()),
            approval: None,
        })
    }

    /// A pane whose runtime is opened answerable (Go: the publish-time
    /// `EnableInteractiveApproval`, driven by the resolved posture).
    fn answerable(approval: reasonix_agent::approval::ApprovalConfig) -> Arc<Self> {
        Arc::new(Self {
            sink: Recorder::shared(),
            kits: Mutex::new(Vec::new()),
            approval: Some(approval),
        })
    }

    fn first_kit(&self) -> RuntimeKit {
        self.kits
            .lock()
            .expect("kit lock")
            .first()
            .cloned()
            .expect("one kit minted per open")
    }
}

impl RuntimeKitFactory for PaneKit {
    fn kit(&self) -> RuntimeKit {
        let kit = RuntimeKit {
            broadcaster: Arc::new(TestBroadcaster),
            sink: self.sink.clone(),
            approval: self.approval,
        };
        self.kits.lock().expect("kit lock").push(kit.clone());
        kit
    }
}

/// A hub with the open machine installed over the given workspace.
fn hub_opening(state_root: &std::path::Path, spec: RuntimeSpec, kit: Arc<PaneKit>) -> Hub {
    Hub::with_options(
        HubPaths {
            state_root: state_root.to_path_buf(),
        },
        HubOptions { spec, kit },
    )
}

fn spec(endpoint: &str, workspace: &std::path::Path) -> RuntimeSpec {
    RuntimeSpec {
        endpoint: endpoint.to_owned(),
        api_key: "k".to_owned(),
        model: "m".to_owned(),
        system_prompt: "You are the hub kernel.".to_owned(),
        workspace_root: workspace.to_string_lossy().into_owned(),
    }
}

#[tokio::test]
async fn open_builds_a_runtime_whose_submit_runs_end_to_end_through_the_hub() {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("proj");
    std::fs::create_dir_all(&workspace).unwrap();
    let pane = PaneKit::shared();
    let base = serve_responses(vec![chunked_sse(&[
        &text_frame("the hub answer"),
        &usage_frame(7, 3, "stop"),
        "[DONE]",
    ])])
    .await;
    let hub = hub_opening(tmp.path(), spec(&base, &workspace), pane.clone());

    let summary = hub
        .open_runtime(OpenRuntimeRequest {
            root: workspace.to_string_lossy().into_owned(),
            ..OpenRuntimeRequest::default()
        })
        .expect("runtime opened");
    assert_eq!(summary.id, "r1");
    assert_eq!(summary.base, "/rt/r1");
    assert_eq!(summary.name, "proj");

    // The frozen read surface names what was published.
    assert_eq!(hub.runtimes(), vec![summary.clone()]);

    // The opened runtime's write side is the hub's TurnController: a
    // submit runs the whole turn and lands on the pane's sink.
    let ctrl = hub
        .turn_controller(RuntimeId("r1".into()))
        .expect("turn controller");
    let receipt = ctrl.submit("hello").await.expect("turn admitted");
    assert!(receipt.error.is_none(), "{:?}", receipt.error);
    let done = pane.sink.events_of(reasonix_session::turns::TURN_DONE);
    assert_eq!(done.len(), 1, "exactly one completion");
    assert!(done[0].err.is_empty());
    let snapshot = pane.sink.snapshot();
    assert!(snapshot.iter().any(|e| e.kind == kind::TEXT));

    // The turn wrote the workspace's own transcript directory under the
    // hub's state root (Go SessionDirFor), opened by the spec's prompt.
    let dir = reasonix_session::paths::session_dir_for(tmp.path(), &summary.root);
    assert_eq!(
        std::path::Path::new(&ctrl.session_path()).parent().unwrap(),
        dir,
        "the transcript lands in the workspace's session dir"
    );
    let bytes = std::fs::read(ctrl.session_path()).expect("transcript");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("hello"), "{text}");
    assert!(text.contains("the hub answer"), "{text}");
    assert!(text.contains("You are the hub kernel."), "{text}");

    // The broadcaster the kit minted is the one the hub hands out.
    let kit = pane.first_kit();
    let handed = hub
        .broadcaster(RuntimeId("r1".into()))
        .expect("broadcaster published");
    assert!(
        Arc::ptr_eq(&handed, &kit.broadcaster),
        "the hub publishes the kit's broadcaster"
    );
}

#[tokio::test]
async fn a_driven_session_refuses_a_second_open_with_the_holder() {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("proj");
    std::fs::create_dir_all(&workspace).unwrap();
    let pane = PaneKit::shared();
    let hub = hub_opening(
        tmp.path(),
        spec("http://127.0.0.1:1", &workspace),
        pane.clone(),
    );
    let session = tmp.path().join("s.jsonl");

    let first = hub
        .open_runtime(OpenRuntimeRequest {
            root: workspace.to_string_lossy().into_owned(),
            session_path: session.to_string_lossy().into_owned(),
            ..OpenRuntimeRequest::default()
        })
        .expect("first open");
    assert_eq!(first.session_path, session.to_string_lossy());

    // One transcript, one writer: the same file cannot bind a second
    // runtime, however it is spelled (Go focuses the holder; this build
    // refuses with it until the pane-focus machine migrates).
    let second = hub.open_runtime(OpenRuntimeRequest {
        root: workspace.to_string_lossy().into_owned(),
        session_path: session.to_string_lossy().into_owned(),
        ..OpenRuntimeRequest::default()
    });
    assert_eq!(
        second,
        Err(HubRefusal::AlreadyOpen {
            id: first.id.clone()
        })
    );
    assert_eq!(hub.runtimes().len(), 1, "no second entry was created");
    assert_eq!(pane.kits.lock().expect("kit lock").len(), 1, "one kit");

    // A workspace without a pinned session opens another pane, as in Go.
    let other = hub
        .open_runtime(OpenRuntimeRequest {
            root: workspace.to_string_lossy().into_owned(),
            ..OpenRuntimeRequest::default()
        })
        .expect("a second pane on one workspace");
    assert_eq!(other.id, "r2");
    assert_eq!(hub.runtimes().len(), 2);
    assert_eq!(hub.runtimes()[0].id, "r1", "publish order is open order");
}

#[tokio::test]
async fn open_refusals_carry_the_real_failure_identities() {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("proj");
    std::fs::create_dir_all(&workspace).unwrap();
    let pane = PaneKit::shared();

    // No endpoint: the spec is incomplete.
    let mut incomplete = spec("http://127.0.0.1:1", &workspace);
    incomplete.endpoint = "  ".to_owned();
    let hub = hub_opening(tmp.path(), incomplete, pane.clone());
    assert_eq!(
        hub.open_runtime(OpenRuntimeRequest {
            root: workspace.to_string_lossy().into_owned(),
            ..OpenRuntimeRequest::default()
        }),
        Err(HubRefusal::MissingField { field: "endpoint" })
    );

    // No model anywhere: the spec and the request are both silent.
    let mut no_model = spec("http://127.0.0.1:1", &workspace);
    no_model.model = String::new();
    let hub = hub_opening(tmp.path(), no_model, pane.clone());
    assert_eq!(
        hub.open_runtime(OpenRuntimeRequest {
            root: workspace.to_string_lossy().into_owned(),
            ..OpenRuntimeRequest::default()
        }),
        Err(HubRefusal::MissingField { field: "model" })
    );

    // No workspace anywhere on an empty hub (Go workspace.none).
    let mut no_workspace = spec("http://127.0.0.1:1", &workspace);
    no_workspace.workspace_root = String::new();
    let hub = hub_opening(tmp.path(), no_workspace, pane.clone());
    assert_eq!(
        hub.open_runtime(OpenRuntimeRequest::default()),
        Err(HubRefusal::NoWorkspace)
    );

    // A named root that does not exist (Go workspace.folder_missing).
    let missing = tmp.path().join("absent");
    let hub = hub_opening(
        tmp.path(),
        spec("http://127.0.0.1:1", &workspace),
        pane.clone(),
    );
    assert_eq!(
        hub.open_runtime(OpenRuntimeRequest {
            root: missing.to_string_lossy().into_owned(),
            ..OpenRuntimeRequest::default()
        }),
        Err(HubRefusal::WorkspaceMissing {
            path: std::path::absolute(&missing)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
    );

    assert_eq!(
        hub.runtimes(),
        Vec::new(),
        "no runtime was published by a refused open"
    );
}

#[tokio::test]
async fn the_request_model_overrides_the_spec_model() {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("proj");
    std::fs::create_dir_all(&workspace).unwrap();
    let pane = PaneKit::shared();
    let base = serve_responses(vec![chunked_sse(&[
        &text_frame("done"),
        &usage_frame(5, 1, "stop"),
        "[DONE]",
    ])])
    .await;
    let hub = hub_opening(tmp.path(), spec(&base, &workspace), pane.clone());

    hub.open_runtime(OpenRuntimeRequest {
        root: workspace.to_string_lossy().into_owned(),
        model: "m2".to_owned(),
        ..OpenRuntimeRequest::default()
    })
    .expect("runtime opened");
    let ctrl = hub
        .turn_controller(RuntimeId("r1".into()))
        .expect("turn controller");
    ctrl.submit("hello").await.expect("turn admitted");

    // The transcript name carries the model that drove the turn
    // (Go: Label — the model name).
    let name = std::path::Path::new(&ctrl.session_path())
        .file_name()
        .expect("transcript name")
        .to_string_lossy()
        .into_owned();
    assert!(name.ends_with("-m2.jsonl"), "{name}");
}

#[tokio::test]
async fn an_open_runtime_without_a_state_root_keeps_the_transcript_in_memory() {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("proj");
    std::fs::create_dir_all(&workspace).unwrap();
    let pane = PaneKit::shared();
    let base = serve_responses(vec![chunked_sse(&[
        &text_frame("done"),
        &usage_frame(5, 1, "stop"),
        "[DONE]",
    ])])
    .await;
    let hub = Hub::with_options(
        HubPaths {
            state_root: std::path::PathBuf::new(),
        },
        HubOptions {
            spec: spec(&base, &workspace),
            kit: pane,
        },
    );
    hub.open_runtime(OpenRuntimeRequest {
        root: workspace.to_string_lossy().into_owned(),
        ..OpenRuntimeRequest::default()
    })
    .expect("runtime opened");
    let ctrl = hub.turn_controller(RuntimeId("r1".into())).unwrap();
    assert!(ctrl.session_dir().is_empty(), "no transcript directory");
    ctrl.submit("hello").await.expect("turn admitted");
    assert!(ctrl.session_path().is_empty(), "nothing was pinned");
    assert!(!ctrl.session().snapshot().is_empty(), "memory holds it");
}

/// The registry the open machine builds drives the write family confined
/// to the opened workspace: the model's write_file lands the file there.
#[tokio::test]
async fn the_opened_registry_drives_the_write_family_in_the_workspace() {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("proj");
    std::fs::create_dir_all(&workspace).unwrap();
    let pane = PaneKit::shared();
    let write_args = format!(
        r#"{{"path":"{}","content":"from the hub runtime"}}"#,
        workspace
            .join("note.txt")
            .to_string_lossy()
            .replace('\\', "/")
    );
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_1", "write_file", &write_args),
            &usage_frame(20, 15, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("written."),
            &usage_frame(40, 6, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let hub = hub_opening(tmp.path(), spec(&base, &workspace), pane);

    hub.open_runtime(OpenRuntimeRequest {
        root: workspace.to_string_lossy().into_owned(),
        ..OpenRuntimeRequest::default()
    })
    .expect("runtime opened");
    let ctrl = hub.turn_controller(RuntimeId("r1".into())).unwrap();
    let receipt = ctrl.submit("write the note").await.expect("turn admitted");
    assert!(receipt.error.is_none(), "{:?}", receipt.error);

    let written = std::fs::read_to_string(workspace.join("note.txt")).expect("written file");
    assert_eq!(written, "from the hub runtime");
}

/// A runtime opened through the hub with a resolved posture is answerable:
/// the model's writer is gated, the human's approval is what lets it run,
/// and the approved file appears (Go: the publish-time
/// `EnableInteractiveApproval`, driven by the resolved config). Two workers
/// because the gate blocks its thread waiting for the decision, exactly as a
/// person-facing prompt does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_with_approval_runs_the_write_only_after_the_answer() {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("proj");
    std::fs::create_dir_all(&workspace).unwrap();
    let pane = PaneKit::answerable(reasonix_agent::approval::ApprovalConfig {
        mode: reasonix_agent::approval::mode::ASK,
        timeout: std::time::Duration::from_secs(5),
    });
    let target = workspace.join("gated.txt");
    let write_args = format!(
        r#"{{"path":"{}","content":"after the answer"}}"#,
        target.to_string_lossy().replace('\\', "/")
    );
    let base = serve_responses(vec![
        chunked_sse(&[
            &tool_call_frame(0, "call_1", "write_file", &write_args),
            &usage_frame(20, 15, "tool_calls"),
            "[DONE]",
        ]),
        chunked_sse(&[
            &text_frame("written."),
            &usage_frame(40, 6, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let hub = hub_opening(tmp.path(), spec(&base, &workspace), pane.clone());

    // The headless default is what an unconfigured spec still gets.
    assert!(
        spec(&base, &workspace).workspace_root == workspace.to_string_lossy()
            && pane.approval.is_some()
    );

    hub.open_runtime(OpenRuntimeRequest {
        root: workspace.to_string_lossy().into_owned(),
        ..OpenRuntimeRequest::default()
    })
    .expect("runtime opened");
    let ctrl = hub.turn_controller(RuntimeId("r1".into())).unwrap();
    assert!(
        ctrl.interactive_approval(),
        "the posture reached the runtime"
    );

    let submit_ctrl = ctrl.clone();
    let submission = tokio::spawn(async move { submit_ctrl.submit("write it").await });
    let id = loop {
        let reqs = pane.sink.events_of("approval_request");
        if let Some(a) = reqs.first().and_then(|e| e.approval.as_ref()) {
            break a.id.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    };
    assert!(!target.exists(), "nothing ran before the human answered");
    assert!(ctrl.approve(
        &id,
        reasonix_agent::approval::DecisionInput {
            allow: true,
            session: false,
            persist: false,
        }
    ));
    let receipt = submission.await.expect("turn task").expect("admitted");
    assert!(receipt.error.is_none(), "{:?}", receipt.error);
    assert_eq!(
        std::fs::read_to_string(&target).expect("approved write"),
        "after the answer"
    );
}

/// The other half of the same rule: a hub whose kit carries no posture opens
/// a headless runtime, and a writer runs with no prompt at all.
#[tokio::test]
async fn an_open_without_a_configured_posture_stays_headless() {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("proj");
    std::fs::create_dir_all(&workspace).unwrap();
    let pane = PaneKit::shared();
    let base = serve_responses(vec![chunked_sse(&[
        &text_frame("nothing to write."),
        &usage_frame(20, 5, "stop"),
        "[DONE]",
    ])])
    .await;
    let hub = hub_opening(tmp.path(), spec(&base, &workspace), pane.clone());
    hub.open_runtime(OpenRuntimeRequest {
        root: workspace.to_string_lossy().into_owned(),
        ..OpenRuntimeRequest::default()
    })
    .expect("runtime opened");
    let ctrl = hub.turn_controller(RuntimeId("r1".into())).unwrap();
    assert!(
        !ctrl.interactive_approval(),
        "an unconfigured open invents no approval"
    );
    assert!(pane.sink.events_of("approval_request").is_empty());
    assert!(ctrl.pending_prompts().is_empty());
}

/// The `ask` tool is part of every opened runtime's registry, whether or not
/// an asker is installed (the schema is cache-stable; a headless run answers
/// "unresolved" from the tool itself).
#[tokio::test]
async fn an_opened_runtime_advertises_the_ask_tool() {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("proj");
    std::fs::create_dir_all(&workspace).unwrap();
    let pane = PaneKit::shared();
    let base = serve_responses(vec![chunked_sse(&[
        &text_frame("ok"),
        &usage_frame(5, 1, "stop"),
        "[DONE]",
    ])])
    .await;
    let hub = hub_opening(tmp.path(), spec(&base, &workspace), pane);
    hub.open_runtime(OpenRuntimeRequest {
        root: workspace.to_string_lossy().into_owned(),
        ..OpenRuntimeRequest::default()
    })
    .expect("runtime opened");
    let ctrl = hub.turn_controller(RuntimeId("r1".into())).unwrap();
    let names: Vec<String> = ctrl.agent().registry().names();
    assert!(names.contains(&"ask".to_owned()), "registry: {names:?}");
    assert!(names.contains(&"write_file".to_owned()));
}
