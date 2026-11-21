//! The fold executor at the run boundary (Go: compact_loop_e2e_test.go's
//! tiny-window oracle, compact_commit_emit_test.go's install half,
//! context_overflow_test.go's replay through the real fold).

mod support;

use std::sync::Arc;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use reasonix_agent::sink::kind;
use reasonix_agent::window::{SharedWindowPolicy, WindowConfig};
use reasonix_contract::provider::Role;
use reasonix_state::sessionstore::Session;
use reasonix_state::sessionstore::compaction_state::load_compaction_state;
use reasonix_tools::{CallContext, Registry, Tool, ToolResult};
use serde_json::Value;
use support::{
    Fixture, chunked_sse, fixture, serve_capturing, serve_responses, text_frame, usage_frame,
};

/// Go: fatTool — a fixed-size blob standing in for a real read whose output
/// dominates the session.
struct FatTool {
    blob: &'static str,
}

impl Tool for FatTool {
    fn name(&self) -> &str {
        "fat_read"
    }
    fn description(&self) -> &str {
        "read a large file"
    }
    fn schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{}})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn execute(&self, _ctx: &CallContext, _args: &Value) -> ToolResult {
        Ok(self.blob.to_owned())
    }
}

fn registry_with_fat_tool(blob: &'static str) -> Arc<Registry> {
    let registry = Registry::new();
    registry.add(Arc::new(FatTool { blob }));
    Arc::new(registry)
}

fn loop_agent_config() -> reasonix_agent::config::AgentConfig {
    reasonix_agent::config::AgentConfig {
        max_steps: 4,
        ..reasonix_agent::config::AgentConfig::default()
    }
}

fn fold_window_config(context_window: i64, archive_dir: &std::path::Path) -> WindowConfig {
    WindowConfig {
        context_window,
        compact_ratio: 0.9,
        model_ref: "relay/m".to_owned(),
        archive_dir: archive_dir.to_string_lossy().into_owned(),
        shared_window: SharedWindowPolicy::default(),
        ..WindowConfig::default()
    }
}

/// One dynamic loop endpoint (Go: loopMock): a tool call while the last
/// message is the user's, a final answer once its result landed. Prompt
/// usage tracks the request size, so calibration follows the real counts.
async fn serve_loop_mock() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let mut buf: Vec<u8> = Vec::new();
            let mut chunk = vec![0u8; 65536];
            loop {
                let n = sock.read(&mut chunk).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if body_complete(&buf) {
                    break;
                }
            }
            let payload = decode_chunked(&buf);
            let body = String::from_utf8_lossy(&payload).into_owned();
            let last_role = last_message_role(&body);
            let prompt = (body.len() as i64) / 4;
            let frames: Vec<String> = if last_role == "tool" {
                vec![
                    text_frame("Done with this step."),
                    usage_frame(prompt, 20, "stop"),
                    "[DONE]".to_owned(),
                ]
            } else {
                vec![
                    support::tool_call_frame(0, "call_1", "fat_read", "{}"),
                    usage_frame(prompt, 20, "tool_calls"),
                    "[DONE]".to_owned(),
                ]
            };
            let frames: Vec<&str> = frames.iter().map(String::as_str).collect();
            let response = chunked_sse(&frames);
            sock.write_all(&response).await.ok();
            sock.shutdown().await.ok();
        }
    });
    format!("http://{addr}")
}

/// Decodes the chunked transfer coding (RFC 9112 §7.1) the transport
/// uploads with.
fn decode_chunked(all: &[u8]) -> Vec<u8> {
    let head_end = all
        .windows(4)
        .position(|w| {
            w == b"

"
        })
        .map(|at| at + 4)
        .unwrap_or(all.len());
    let mut rest = &all[head_end..];
    let mut out = Vec::new();
    while let Some(line_end) = rest.windows(2).position(|w| {
        w == b"
"
    }) {
        let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..line_end]).trim(), 16)
            .unwrap_or(0);
        rest = &rest[line_end + 2..];
        if size == 0 {
            break;
        }
        let take = size.min(rest.len());
        out.extend_from_slice(&rest[..take]);
        rest = &rest[take..];
        if rest.starts_with(
            b"
",
        ) {
            rest = &rest[2..];
        }
    }
    out
}

/// Whether the bytes accumulated so far complete an HTTP upload: a chunked
/// body ends at its zero terminator, a sized body at its declared length.
fn body_complete(all: &[u8]) -> bool {
    let head_end = all
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|at| at + 4)
        .unwrap_or(all.len());
    let head = String::from_utf8_lossy(&all[..head_end]).into_owned();
    if head
        .lines()
        .any(|l| l.eq_ignore_ascii_case("transfer-encoding: chunked"))
    {
        return all.ends_with(b"0\r\n\r\n");
    }
    let len = head
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    len > 0 && all.len() >= head_end + len
}

fn last_message_role(body: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return String::new();
    };
    v.get("messages")
        .and_then(|m| m.as_array())
        .and_then(|msgs| msgs.last())
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_owned()
}

/// A scratch root for one test's archive tree and session file.
fn scratch(name: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "reasonix-fold-e2e-{}-{}-{}",
        name,
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

fn installed_folds(fixture: &Fixture) -> usize {
    fixture
        .recorder
        .events_of(kind::COMPACTION_DONE)
        .iter()
        .filter(|e| e.compaction.as_ref().is_some_and(|c| !c.summary.is_empty()))
        .count()
}

// Go: TestTinyWindowFoldsEveryRoundAndStaysBounded — a tiny window whose
// every tool result nearly fills it is archived each round instead of
// blocking; what the fold owes is a request that stays under the window.
#[tokio::test]
async fn a_tiny_window_folds_every_round_and_stays_bounded() {
    let base = serve_loop_mock().await;
    let blob: &'static str = Box::leak("LARGE FILE CONTENTS. ".repeat(350).into_boxed_str());
    let registry = registry_with_fat_tool(blob);
    let Fixture {
        agent,
        session,
        recorder,
    } = support::fixture_with_config(&base, registry.clone(), loop_agent_config()).await;
    let dir = scratch("tiny");
    let path = dir.join("s.jsonl");
    let window = Arc::new(reasonix_agent::Window::new(
        fold_window_config(1600, &dir),
        session.clone(),
        registry.clone(),
        recorder.clone() as Arc<dyn reasonix_agent::Sink>,
    ));
    reasonix_agent::window::fold::ArchiveFold::install(&window);
    window.set_session_path(&path.to_string_lossy());
    agent.set_window(window.clone());

    for i in 0..3 {
        agent
            .run(&format!("turn {i}: keep going, continue the work"))
            .await
            .expect("turn completes");
    }
    let _ = &recorder;

    let folds = installed_folds(&Fixture {
        agent: agent.clone(),
        recorder: recorder.clone(),
        session: session.clone(),
    });
    assert!(
        folds >= 2,
        "folds completed = {folds}, want a fold on every round"
    );
    // No fold landed above the trigger or stayed blocked: the tiny window
    // stays bounded, never stuck.
    let blocked = recorder
        .events_of(kind::CONTEXT_MAINTENANCE)
        .iter()
        .filter(|e| {
            e.maintenance
                .as_ref()
                .is_some_and(|m| m.status == "blocked")
        })
        .count();
    assert_eq!(
        blocked, 0,
        "a tiny-window fold was blocked: the fold loop jammed"
    );
    assert!(
        window.current_projection_version() > 0,
        "no projection was installed"
    );
    // The canonical transcript is never rewritten: the original request
    // still opens it, and the turn appended every round to it.
    let canonical = session.snapshot();
    assert_eq!(
        canonical[0].content,
        "turn 0: keep going, continue the work"
    );
    assert!(session.len() > 12, "the transcript did not grow");
    // The archive lives beside the bound transcript.
    let out = reasonix_state::store::naming::session_outputs_dir(&path);
    let files = std::fs::read_dir(&out).expect("outputs dir").count();
    assert!(
        files >= 1,
        "no archive file was written beside the transcript"
    );
    // The sidecar on disk carries the applied receipt.
    for e in std::fs::read_dir(&dir).expect("dir") {
        eprintln!("DIR {}", e.expect("entry").file_name().to_string_lossy());
    }
    // load_compaction_state derives the context path itself.
    let sidecar = load_compaction_state(&path)
        .expect("load")
        .expect("sidecar");
    assert_eq!(sidecar.last_receipt.expect("receipt").status, "applied");
}

// The installed fold restores from the sidecar on a rebind and splices the
// canonical tail behind itself (Go: New's LoadProjectionSidecar resume
// path, projection_valid_test.go).
#[tokio::test]
async fn an_installed_fold_restores_from_the_sidecar_on_rebind() {
    let base = serve_loop_mock().await;
    let blob: &'static str = Box::leak("LARGE FILE CONTENTS. ".repeat(350).into_boxed_str());
    let registry = registry_with_fat_tool(blob);
    let Fixture { agent, session, .. } =
        support::fixture_with_config(&base, registry.clone(), loop_agent_config()).await;
    let dir = scratch("restore");
    let path = dir.join("s.jsonl");
    let window = Arc::new(reasonix_agent::Window::new(
        fold_window_config(1600, &dir),
        session.clone(),
        registry.clone(),
        Arc::new(reasonix_agent::Discard),
    ));
    reasonix_agent::window::fold::ArchiveFold::install(&window);
    window.set_session_path(&path.to_string_lossy());
    agent.set_window(window.clone());

    agent.run("turn 0: keep going").await.expect("turn");
    let version = window.current_projection_version();
    assert!(version > 0, "no fold installed");

    // A new window over the same transcript restores the fold.
    let restored = Arc::new(reasonix_agent::Window::new(
        fold_window_config(1600, &dir),
        session.clone(),
        registry,
        Arc::new(reasonix_agent::Discard),
    ));
    restored.load_projection_sidecar(&path.to_string_lossy());
    assert_eq!(restored.current_projection_version(), version);
    assert_eq!(
        restored.projection_dropped(),
        "",
        "a valid fold reported dropped"
    );
    // The restored view carries the digest plus the canonical tail.
    let visible = restored.model_visible_messages();
    assert!(
        visible
            .iter()
            .any(|m| m.role == Role::USER && m.content.contains("<compaction-summary>")),
        "restored view lost the digest: {visible:?}"
    );
    // The restored window sees exactly what the folding window saw: the
    // frozen body plus the same canonical tail.
    assert_eq!(
        visible.len(),
        window.model_visible_messages().len(),
        "restored view diverged from the folding window's"
    );
}

fn overflow_rejection_body() -> &'static str {
    r#"{"error":{"message":"this model's maximum context length is 32768 tokens","type":"invalid_request_error","code":"context_length_exceeded"}}"#
}

fn http_rejection() -> Vec<u8> {
    let body = overflow_rejection_body();
    format!(
        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes()
}

fn preload(session: &Session) {
    let big = "word ".repeat(400);
    let mut msgs = vec![
        Message {
            role: Role::SYSTEM,
            content: "sys".to_owned(),
            ..Message::default()
        },
        Message {
            role: Role::USER,
            content: "standing constraint: never change the public API".to_owned(),
            ..Message::default()
        },
    ];
    for _ in 0..20 {
        msgs.push(Message {
            role: Role::ASSISTANT,
            content: big.clone(),
            ..Message::default()
        });
        msgs.push(Message {
            role: Role::USER,
            content: "continue".to_owned(),
            ..Message::default()
        });
    }
    session.replace(msgs);
}

use reasonix_contract::provider::Message;

fn request_content_chars(body: &str) -> i64 {
    let v: serde_json::Value = serde_json::from_str(body).expect("request body is JSON");
    v.get("messages")
        .and_then(|m| m.as_array())
        .map(|msgs| {
            msgs.iter()
                .map(|m| {
                    m.get("content")
                        .and_then(|c| c.as_str())
                        .map(|s| s.len() as i64)
                        .unwrap_or(0)
                })
                .sum()
        })
        .unwrap_or(0)
}

// Go: TestContextOverflowRejectionFoldsAndReplays through the real fold: a
// provider that refuses the request as too long gets a smaller one, not a
// failed turn.
#[tokio::test]
async fn an_overflow_rejection_folds_through_the_real_executor() {
    let (base, bodies) = serve_capturing(vec![
        http_rejection(),
        chunked_sse(&[
            &text_frame("recovered"),
            &usage_frame(7, 3, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let registry = Arc::new(Registry::new());
    for tool in reasonix_tools::builtin::unconfined_readers() {
        registry.add(tool);
    }
    let Fixture { agent, session, .. } = fixture(&base, registry.clone()).await;
    preload(&session);

    let dir = scratch("overflow");
    let window = Arc::new(reasonix_agent::Window::new(
        fold_window_config(40_000, &dir),
        session.clone(),
        registry,
        Arc::new(reasonix_agent::Discard),
    ));
    reasonix_agent::window::fold::ArchiveFold::install(&window);
    agent.set_window(window.clone());

    let out = agent.run("continue").await.expect("turn recovers");
    assert_eq!(out.text, "recovered");

    let bodies = bodies.lock().expect("bodies");
    assert_eq!(bodies.len(), 2, "provider saw refused + replay");
    let refused = request_content_chars(&bodies[0]);
    let replayed = request_content_chars(&bodies[1]);
    assert!(
        replayed < refused,
        "replay carried {replayed} against the refused {refused}; the fold freed nothing"
    );
    // The fold installed through the seam, not a test stub.
    assert!(window.current_projection_version() > 0);
    let _ = dir;
}

// Go: TestManualCompactWithNothingToFoldIsAVerdictNotAnOverflow (loop half)
// — a prepare on a short transcript stays a verdict for the caller.
#[tokio::test]
async fn prepare_on_a_short_transcript_answers_without_error() {
    let base = serve_responses(vec![]).await;
    let registry = Arc::new(Registry::new());
    let Fixture { session, .. } = fixture(&base, registry.clone()).await;
    session.replace(vec![Message {
        role: Role::SYSTEM,
        content: "sys".to_owned(),
        ..Message::default()
    }]);
    let dir = scratch("short");
    let window = Arc::new(reasonix_agent::Window::new(
        WindowConfig {
            context_window: 200_000,
            compact_ratio: 0.8,
            model_ref: "relay/m".to_owned(),
            archive_dir: dir.to_string_lossy().into_owned(),
            ..WindowConfig::default()
        },
        session.clone(),
        registry,
        Arc::new(reasonix_agent::Discard),
    ));
    reasonix_agent::window::fold::ArchiveFold::install(&window);
    let prepared = window
        .prepare(reasonix_agent::window::ContextPreparePolicy {
            trigger: "manual".to_owned(),
            ignore_threshold: true,
            ..reasonix_agent::window::ContextPreparePolicy::default()
        })
        .expect("prepare answers");
    assert!(
        !prepared.maintenance.installed,
        "a three-line transcript folded"
    );
    assert!(
        !prepared.maintenance.reason.is_empty(),
        "a declined request carries no reason"
    );
}
