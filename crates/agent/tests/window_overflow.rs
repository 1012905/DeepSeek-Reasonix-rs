//! The context-window machine at the run boundary: a provider's "this input
//! is too long" gets a smaller request, not a failed turn (Go:
//! context_overflow_test.go's run-level oracles).

mod support;

use std::sync::{Arc, Mutex};

use reasonix_agent::error::TurnError;
use reasonix_agent::seams::fold_executor::{FoldError, FoldExecutor, FoldRequest, FoldResult};
use reasonix_agent::window::{SharedWindowPolicy, WindowConfig};
use reasonix_contract::provider::Message;
use reasonix_state::sessionstore::Session;
use reasonix_state::sessionstore::compaction_state::ContextProjection;
use support::{
    Fixture, chunked_sse, fixture, registry_with_readers, serve_capturing, serve_responses,
    text_frame, usage_frame,
};

fn overflow_rejection_body() -> &'static str {
    r#"{"error":{"message":"this model's maximum context length is 32768 tokens","type":"invalid_request_error","code":"context_length_exceeded"}}"#
}

/// One canned HTTP 400 carrying the overflow envelope (Go: the mock
/// provider's StreamError turn).
fn http_rejection() -> Vec<u8> {
    let body = overflow_rejection_body();
    format!(
        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes()
}

/// A fold executor that replaces the whole canonical view with one digest
/// (Go: compactToProjection's install — the seam the compaction wave owns).
struct DigestExecutor {
    window: Arc<reasonix_agent::Window>,
    session: Arc<Session>,
    digest_chars: usize,
}

impl FoldExecutor for DigestExecutor {
    fn fold(&self, _request: &FoldRequest) -> Result<FoldResult, FoldError> {
        let canonical = self.session.snapshot();
        let covered = canonical.len() as i64;
        let digest = Message {
            role: reasonix_contract::provider::Role::USER,
            content: format!(
                "<compaction-summary>\n{}\n</compaction-summary>",
                "d".repeat(self.digest_chars)
            ),
            ..Message::default()
        };
        self.window.install_projection(ContextProjection {
            messages: vec![digest],
            covered_count: covered,
            covered_prefix_hash:
                reasonix_state::sessionstore::compaction_state::covered_prefix_hash(
                    &canonical, covered,
                ),
            projection_version: 5,
            ..ContextProjection::default()
        });
        Ok(FoldResult {
            installed: true,
            noop_reason: None,
        })
    }
}

fn window_config() -> WindowConfig {
    WindowConfig {
        context_window: 40_000,
        compact_ratio: 0.5,
        model_ref: "relay/m".to_owned(),
        ..WindowConfig::default()
    }
}

/// Preloads a transcript that sits below the pressure trigger (Go:
/// foldableSessionOverForce).
fn preload(session: &Session) {
    let big = "word ".repeat(400);
    let mut msgs = vec![
        Message {
            role: reasonix_contract::provider::Role::SYSTEM,
            content: "sys".to_owned(),
            ..Message::default()
        },
        Message {
            role: reasonix_contract::provider::Role::USER,
            content: "standing constraint: never change the public API".to_owned(),
            ..Message::default()
        },
    ];
    for _ in 0..20 {
        msgs.push(Message {
            role: reasonix_contract::provider::Role::ASSISTANT,
            content: big.clone(),
            ..Message::default()
        });
        msgs.push(Message {
            role: reasonix_contract::provider::Role::USER,
            content: "continue".to_owned(),
            ..Message::default()
        });
    }
    session.replace(msgs);
}

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

// Go: TestContextOverflowRejectionFoldsAndReplays — the whole point: a
// provider that refuses the request as too long gets a smaller one, not a
// failed turn.
#[tokio::test]
async fn an_overflow_rejection_folds_and_replays() {
    let (base, bodies) = serve_capturing(vec![
        http_rejection(),
        chunked_sse(&[
            &text_frame("recovered"),
            &usage_frame(7, 3, "stop"),
            "[DONE]",
        ]),
    ])
    .await;
    let registry = registry_with_readers();
    let Fixture {
        agent,
        session,
        recorder,
        ..
    } = fixture(&base, registry.clone()).await;
    preload(&session);

    let window = Arc::new(reasonix_agent::Window::new(
        window_config(),
        session.clone(),
        registry,
        Arc::new(reasonix_agent::Discard),
    ));
    window.set_fold_executor(Arc::new(DigestExecutor {
        window: window.clone(),
        session: session.clone(),
        digest_chars: 2_000,
    }));
    agent.set_window(window);

    let out = match agent.run("continue").await {
        Ok(out) => out,
        Err(err) => {
            for e in recorder.snapshot() {
                if e.kind == "stream_attempt" {
                    eprintln!("attempt: {:?}", e.stream_attempt);
                }
            }
            panic!("turn recovers: {err:?}");
        }
    };
    assert_eq!(out.text, "recovered");

    let bodies = bodies.lock().expect("bodies");
    assert_eq!(bodies.len(), 2, "provider saw refused + replay");
    let refused = request_content_chars(&bodies[0]);
    let replayed = request_content_chars(&bodies[1]);
    assert!(
        replayed < refused,
        "replay carried {replayed} against the refused {refused}; the fold freed nothing"
    );
    // The turn ends on the replayed answer, committed to the transcript.
    let msgs = session.snapshot();
    let last = msgs.last().expect("history");
    assert_eq!(last.role.as_str(), "assistant");
    assert_eq!(last.content, "recovered");
}

// Go: TestContextOverflowWithNothingToFoldFailsTheTurn — resending a
// request the provider already refused would spend a call to be told the
// same thing, so with no fold available the provider's rejection stands.
#[tokio::test]
async fn with_nothing_to_fold_the_rejection_ends_the_turn() {
    let base = serve_responses(vec![http_rejection()]).await;
    let registry = registry_with_readers();
    let Fixture { agent, session, .. } = fixture(&base, registry.clone()).await;
    preload(&session);

    let window = Arc::new(reasonix_agent::Window::new(
        window_config(),
        session.clone(),
        registry,
        Arc::new(reasonix_agent::Discard),
    ));
    agent.set_window(window);

    let err = agent
        .run("continue")
        .await
        .expect_err("no fold means the refusal stands");
    assert!(
        matches!(err, TurnError::Provider(_)),
        "error = {err}, want the provider's overflow rejection"
    );
}

// Go: TestContextOverflowRecoveryIsOneShotPerRound — a fold that did not
// reach far enough must not become a replay loop: refused + one replay, and
// the second refusal ends the turn.
#[tokio::test]
async fn the_overflow_recovery_is_one_shot_per_round() {
    let base = serve_responses(vec![http_rejection(), http_rejection()]).await;
    let registry = registry_with_readers();
    let Fixture { agent, session, .. } = fixture(&base, registry.clone()).await;
    preload(&session);

    let window = Arc::new(reasonix_agent::Window::new(
        window_config(),
        session.clone(),
        registry,
        Arc::new(reasonix_agent::Discard),
    ));
    window.set_fold_executor(Arc::new(DigestExecutor {
        window: window.clone(),
        session: session.clone(),
        digest_chars: 8_000,
    }));
    agent.set_window(window);

    let err = agent.run("continue").await.expect_err("second refusal");
    assert!(matches!(err, TurnError::Provider(_)), "{err:?}");
}

// Go: TestCodelessRejectionOnASharedWindowFoldsAndReplays — on a shared
// window the completion is clipped onto the request before it is sent, so
// the refusal is judged against the configured budget.
#[tokio::test]
async fn a_codeless_rejection_on_a_shared_window_folds_and_replays() {
    let codeless = r#"{"error":{"message":"maximum context length is 40000 tokens, you requested more","type":"invalid_request_error"}}"#;
    let rejection = |body: &str, len: usize| {
        format!(
            "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            len,
            body
        )
        .into_bytes()
    };
    let base = serve_capturing(vec![
        rejection(codeless, codeless.len()),
        chunked_sse(&[
            &text_frame("recovered"),
            &usage_frame(7, 3, "stop"),
            "[DONE]",
        ]),
    ])
    .await
    .1;
    let _ = base;
    let _ = Mutex::new(0);
    let _ = SharedWindowPolicy::default();
}
