//! Canned provider, recorder sink, and fixtures the turn-level tests share
//! (the provider crate's transport-test pattern, extended to answer
//! several sequential connections). Each test binary compiles this module
//! separately and uses a different slice of it.

#![forbid(unsafe_code)]
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use reasonix_agent::config::AgentConfig;
use reasonix_agent::sink::{Sink, kind};
use reasonix_contract::eventwire::event::Event;
use reasonix_provider::config::{ClientConfig, build_client};
use reasonix_state::sessionstore::Session;
use reasonix_tools::Registry;

/// HTTP/1.1 chunked SSE response around the given data frames.
pub fn chunked_sse(frames: &[&str]) -> Vec<u8> {
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

/// One SSE delta frame carrying a text chunk.
pub fn text_frame(t: &str) -> String {
    let escaped = t.replace('\\', "\\\\").replace('"', "\\\"");
    format!(r#"{{"choices":[{{"delta":{{"content":"{escaped}"}}}}]}}"#)
}

/// One SSE frame completing a tool call at the given index. Distinct calls
/// in one reply must carry distinct indexes: the decoder accumulates deltas
/// by index, so two calls sharing one merge their arguments.
pub fn tool_call_frame(index: usize, id: &str, name: &str, arguments: &str) -> String {
    let escaped = arguments.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        r#"{{"choices":[{{"delta":{{"tool_calls":[{{"index":{index},"id":"{id}","function":{{"name":"{name}","arguments":"{escaped}"}}}}]}}}}]}}"#
    )
}

/// One SSE usage frame with a finish reason.
pub fn usage_frame(prompt: i64, completion: i64, finish: &str) -> String {
    format!(
        r#"{{"choices":[{{"delta":{{}},"finish_reason":"{finish}"}}],"usage":{{"prompt_tokens":{prompt},"completion_tokens":{completion},"total_tokens":{}}}}}"#,
        prompt + completion
    )
}

/// One-shot HTTP server answering each accepted connection with the next
/// canned response, then closing. A test's provider makes one connection
/// per stream call, so the responses list is the round sequence.
pub async fn serve_responses(responses: Vec<Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for response in responses {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let mut buf = vec![0u8; 65536];
            // One drain of the request is enough for the canned exchange;
            // the transport only needs the connection accepted.
            let _ = sock.read(&mut buf).await;
            sock.write_all(&response).await.ok();
            sock.shutdown().await.ok();
        }
    });
    format!("http://{addr}")
}

/// Collects every event the spine emits, in emit order.
#[derive(Default)]
pub struct Recorder {
    pub events: Mutex<Vec<Event>>,
}

impl Recorder {
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn snapshot(&self) -> Vec<Event> {
        self.events.lock().expect("recorder lock").clone()
    }

    pub fn kinds(&self) -> Vec<String> {
        self.snapshot().into_iter().map(|e| e.kind).collect()
    }

    pub fn text(&self) -> String {
        self.snapshot()
            .into_iter()
            .filter(|e| e.kind == kind::TEXT)
            .map(|e| e.text)
            .collect()
    }

    pub fn events_of(&self, kind_name: &str) -> Vec<Event> {
        self.snapshot()
            .into_iter()
            .filter(|e| e.kind == kind_name)
            .collect()
    }
}

impl Sink for Recorder {
    fn emit(&self, e: Event) {
        self.events.lock().expect("recorder lock").push(e);
    }
}

/// One agent over the canned endpoint with the standard fixtures. The
/// registry carries the unconfined read tools; a test studying writes
/// builds its own registry (see turn_boundaries.rs registry_with_writers).
pub struct Fixture {
    pub agent: Arc<reasonix_agent::run::Agent>,
    pub recorder: Arc<Recorder>,
    pub session: Arc<Session>,
}

pub async fn fixture(base: &str, registry: Arc<Registry>) -> Fixture {
    fixture_with_config(base, registry, AgentConfig::default()).await
}

/// A fixture over an explicit config. Empty fields keep the standard
/// fixture values, so a test overrides exactly what it studies.
pub async fn fixture_with_config(
    base: &str,
    registry: Arc<Registry>,
    config: AgentConfig,
) -> Fixture {
    let mut cfg = ClientConfig::new("relay", base, "m");
    cfg.api_key = "k".to_owned();
    let client = build_client(&cfg).expect("build client");
    let mut config = config;
    if config.model_ref.is_empty() {
        config.model_ref = "relay/m".to_owned();
    }
    if config.system_prompt.is_empty() {
        config.system_prompt = "You are the test kernel.".to_owned();
    }
    if config.retry_backoff.is_none() {
        config.retry_backoff = Some(Duration::ZERO);
    }
    let recorder = Recorder::shared();
    let session = Arc::new(Session::new(""));
    let agent = reasonix_agent::Agent::new(
        config,
        client,
        registry,
        session.clone(),
        recorder.clone() as Arc<dyn Sink>,
    );
    Fixture {
        agent: Arc::new(agent),
        recorder,
        session,
    }
}

pub fn registry_with_readers() -> Arc<Registry> {
    let registry = Registry::new();
    for tool in reasonix_tools::builtin::unconfined_readers() {
        registry.add(tool);
    }
    Arc::new(registry)
}
