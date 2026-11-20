//! The bridge the agent loop emits through (the agent spine's integration
//! note, kept next to the pipeline it wraps): the boot-composed emit
//! pipeline is a [`Sink`](crate::sink::Sink) carrying the frontend
//! capability channels, and the agent loop's
//! [`reasonix_agent::sink::Sink`] is the bare `emit` every emitter shares.
//! Boot wraps the broadcaster once, and this adapter is what lets the turn
//! machine — and any other kernel emitter — capture that wrapped chain
//! without the agent crate reaching a frontend.

use std::sync::Arc;

use reasonix_agent::sink::Sink as AgentSink;
use reasonix_contract::eventwire::event::Event;

use crate::sink::Sink;

/// An agent-side sink over the composed emit pipeline (Go build.go:239 —
/// the broadcaster sits at the pipeline's end and every emitter shares the
/// chain). The capability channels stay on the serve side; the agent emits
/// plain events only.
pub struct PipelineSink {
    pipeline: Arc<dyn Sink>,
}

impl PipelineSink {
    /// Wraps the boot-composed pipeline.
    pub fn new(pipeline: Arc<dyn Sink>) -> Self {
        Self { pipeline }
    }

    /// The wrapped pipeline, for hosts that also emit through it directly.
    pub fn pipeline(&self) -> &Arc<dyn Sink> {
        &self.pipeline
    }
}

impl AgentSink for PipelineSink {
    fn emit(&self, e: Event) {
        self.pipeline.emit(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::{DEFAULT_STREAM_DELTA_WINDOW, coalesce, sync_sink};
    use reasonix_agent::config::AgentConfig;
    use reasonix_contract::provider::Role;
    use reasonix_provider::config::{ClientConfig, build_client};
    use reasonix_state::sessionstore::Session;
    use reasonix_tools::Registry;
    use std::time::Duration;

    /// The recording terminal sink (Go: the frontend sink a test replaces).
    #[derive(Default)]
    struct Recorder {
        events: std::sync::Mutex<Vec<Event>>,
    }

    impl Recorder {
        fn shared() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn kinds(&self) -> Vec<String> {
            self.events
                .lock()
                .expect("recorder lock")
                .iter()
                .map(|e| e.kind.clone())
                .collect()
        }

        fn texts(&self) -> String {
            self.events
                .lock()
                .expect("recorder lock")
                .iter()
                .filter(|e| e.kind == "text")
                .map(|e| e.text.clone())
                .collect()
        }
    }

    impl Sink for Recorder {
        fn emit(&self, e: Event) {
            self.events.lock().expect("recorder lock").push(e);
        }
    }

    fn turn_started() -> Event {
        Event {
            kind: "turn_started".to_owned(),
            text: "hello".to_owned(),
            ..Event::default()
        }
    }

    fn text(t: &str) -> Event {
        Event {
            kind: "text".to_owned(),
            text: t.to_owned(),
            ..Event::default()
        }
    }

    #[tokio::test]
    async fn the_adapter_delivers_events_through_the_pipeline_in_order() {
        let recorder = Recorder::shared();
        // Boot order minus the machines not yet ported: coalesce → sync →
        // terminal (Go: mark_embedded_diffs(coalesce(sync(...)))).
        let pipeline = coalesce(
            sync_sink(recorder.clone() as Arc<dyn Sink>),
            DEFAULT_STREAM_DELTA_WINDOW,
        );
        let agent_sink: Arc<dyn AgentSink> = Arc::new(PipelineSink::new(pipeline));
        agent_sink.emit(turn_started());
        agent_sink.emit(text("a"));
        agent_sink.emit(text("b"));
        agent_sink.emit(Event {
            kind: reasonix_session::turns::TURN_DONE.to_owned(),
            ..Event::default()
        });
        let kinds = recorder.kinds();
        assert_eq!(kinds.first().map(String::as_str), Some("turn_started"));
        assert_eq!(kinds.last().map(String::as_str), Some("turn_done"));
        assert!(
            kinds.iter().all(|k| k != "reasoning"),
            "only what was emitted arrives"
        );
    }

    // Ported assembly order Coalesce → Sync → [Broadcaster] (Go build.go:239
    // with the recording sink as the frontend): a delta burst merges before
    // the boundary event through the agent bridge.
    #[tokio::test(start_paused = true)]
    async fn a_turns_delta_burst_merges_before_the_boundary() {
        let recorder = Recorder::shared();
        let pipeline = coalesce(
            sync_sink(recorder.clone() as Arc<dyn Sink>),
            DEFAULT_STREAM_DELTA_WINDOW,
        );
        let agent_sink: Arc<dyn AgentSink> = Arc::new(PipelineSink::new(pipeline));
        agent_sink.emit(text("a"));
        agent_sink.emit(text("b"));
        agent_sink.emit(text("c"));
        agent_sink.emit(Event {
            kind: reasonix_session::turns::TURN_DONE.to_owned(),
            ..Event::default()
        });
        // The deltas merged into one text event, then the boundary.
        assert_eq!(recorder.texts(), "abc");
    }

    fn canned_sse_response(frames: &[String]) -> Vec<u8> {
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

    /// A real turn's events reach the recorder through the bridge (Go: the
    /// emitter the boot composes once, driven here by the turn machine).
    #[tokio::test]
    async fn a_turns_events_reach_the_pipeline_through_the_bridge() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let mut buf = vec![0u8; 65536];
            let _ = sock.read(&mut buf).await;
            let body = canned_sse_response(&[
                r#"{"choices":[{"delta":{"content":"streamed answer"}}]}"#.to_owned(),
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#.to_owned(),
                "[DONE]".to_owned(),
            ]);
            sock.write_all(&body).await.ok();
            sock.shutdown().await.ok();
        });
        let mut cfg = ClientConfig::new("relay", &format!("http://{addr}"), "m");
        cfg.api_key = "k".to_owned();
        let client = build_client(&cfg).expect("build client");

        let recorder = Recorder::shared();
        let pipeline = coalesce(
            sync_sink(recorder.clone() as Arc<dyn Sink>),
            DEFAULT_STREAM_DELTA_WINDOW,
        );
        let agent_sink: Arc<dyn AgentSink> = Arc::new(PipelineSink::new(pipeline));
        let agent = reasonix_agent::Agent::new(
            AgentConfig {
                model_ref: "relay/m".to_owned(),
                system_prompt: "sys".to_owned(),
                retry_backoff: Some(Duration::ZERO),
                ..AgentConfig::default()
            },
            client,
            Arc::new(Registry::new()),
            // The controller-shaped session: the transcript opens with the
            // cache-stable prompt (Go: NewSession(b.prompt.prompt)), and the
            // composer leaves it alone.
            Arc::new(Session::new("sys")),
            agent_sink,
        );
        let out = agent.run("a prompt").await.expect("turn");
        assert!(out.text.contains("streamed answer"));

        let kinds = recorder.kinds();
        assert_eq!(kinds.first().map(String::as_str), Some("turn_started"));
        assert!(kinds.contains(&"text".to_owned()));
        assert!(kinds.contains(&"usage".to_owned()));
        assert_eq!(recorder.texts(), "streamed answer");
        let msgs = agent.session().snapshot();
        assert_eq!(msgs[0].role, Role::SYSTEM);
        assert_eq!(msgs[0].content, "sys");
    }
}
