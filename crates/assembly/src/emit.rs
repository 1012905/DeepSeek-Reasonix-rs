//! The emit chain boot composes once per runtime (Go `build.go:239`): the
//! broadcaster is wrapped in the pipeline before any emitter captures it,
//! so coalescing, diff tagging and serialization happen once for every
//! frontends. The agent loop takes the composed chain through the serve
//! adapter — the kernel never reaches a frontend type from below.

use std::sync::Arc;

use reasonix_agent::approval::ApprovalConfig;
use reasonix_agent::sink::Sink as AgentSink;
use reasonix_serve::broadcaster::Broadcaster;
use reasonix_serve::sink;
use reasonix_session::hub::{RuntimeKit, RuntimeKitFactory};

/// Composes the runtime's emit chain over its broadcaster and hands back
/// the sink the turn machine emits through (Go: the
/// `mark_embedded_diffs(goal_tee(coalesce(sync(cost_quote_sink(broadcaster)))))
/// chain, built once and shared by the controller, the agent loop and the
/// background emitters). The goal tee and the cost quote join where they
/// sit in Go's order when their machines migrate — the goal FSM and the
/// pricing context boot resolves.
pub fn turn_emit_pipeline(broadcaster: Arc<Broadcaster>, mark_diffs: bool) -> Arc<dyn AgentSink> {
    let terminal: Arc<dyn sink::Sink> = broadcaster;
    let pipeline = sink::coalesce(sink::sync_sink(terminal), sink::DEFAULT_STREAM_DELTA_WINDOW);
    let pipeline = sink::mark_embedded_diffs(pipeline, mark_diffs);
    Arc::new(sink::PipelineSink::new(pipeline))
}

/// The pane kit factory (session's [`RuntimeKitFactory`]): the broadcaster
/// and the emit chain over it, minted fresh per open — Go `openBound`'s
/// `bc := NewBroadcaster()` + `paneSink := h.decorateSink(bc)`. This is the
/// collaborator the host installs on the hub's open machine; the runtime
/// spec (endpoint/key/model/prompt) comes from the serve config edge.
///
/// `approval` is the posture the resolved configuration named, or `None`
/// when it named none — the headless default. Booleans are not a posture:
/// the value comes from [reasonix_config::Config]'s own accessor, so a
/// setting this host does not recognise cannot silently open a prompt the
/// user never asked for, or approve a call they never saw.
pub struct PaneKit {
    mark_diffs: bool,
    approval: Option<ApprovalConfig>,
}

impl PaneKit {
    pub fn new(mark_diffs: bool) -> Self {
        Self {
            mark_diffs,
            approval: None,
        }
    }

    /// Mints kits carrying the resolved approval posture (Go: the host
    /// completing `EnableInteractiveApproval` at publish time, because that
    /// is where the posture is known).
    pub fn with_approval(mark_diffs: bool, approval: Option<ApprovalConfig>) -> Self {
        Self {
            mark_diffs,
            approval,
        }
    }
}

impl RuntimeKitFactory for PaneKit {
    fn kit(&self) -> RuntimeKit {
        let broadcaster = Arc::new(Broadcaster::new());
        let handle = broadcaster.clone();
        RuntimeKit {
            broadcaster: handle,
            sink: turn_emit_pipeline(broadcaster, self.mark_diffs),
            approval: self.approval,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reasonix_serve::broadcaster::SubscriptionMode;
    use reasonix_serve::sink::Event;

    fn text(t: &str) -> Event {
        Event {
            kind: "text".to_owned(),
            text: t.to_owned(),
            ..Event::default()
        }
    }

    // Boot order mark_embedded_diffs → coalesce → sync → [Broadcaster] (Go
    // build.go:239): the agent's events reach a subscriber through the
    // whole chain, in emit order, with the delta burst merged.
    #[tokio::test(start_paused = true)]
    async fn the_turn_pipeline_reaches_a_subscriber_through_the_whole_chain() {
        let broadcaster = Arc::new(Broadcaster::new());
        let mut sub = broadcaster.subscribe(SubscriptionMode::From(0));
        let agent_sink = turn_emit_pipeline(broadcaster.clone(), false);
        agent_sink.emit(text("a"));
        agent_sink.emit(text("b"));
        agent_sink.emit(text("c"));
        agent_sink.emit(Event {
            kind: reasonix_session::turns::TURN_DONE.to_owned(),
            ..Event::default()
        });
        // The burst's leading delta forwards at once (the coalescer's
        // latency rule); the rest merge ahead of the boundary. Drain to the
        // completion and compare the whole text.
        let mut texts = String::new();
        let mut saw_done = false;
        for _ in 0..8 {
            let frame = sub.recv().await.expect("frame");
            let w: serde_json::Value = serde_json::from_slice(&frame.data).unwrap();
            match w["kind"].as_str() {
                Some("text") => texts.push_str(w["text"].as_str().unwrap()),
                Some("turn_done") => {
                    saw_done = true;
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(texts, "abc", "the burst merges ahead of the boundary");
        assert!(saw_done, "the completion reaches the subscriber");
    }

    // A chained sink is one emitter share: emitting through the serve-side
    // pipeline handle (what the controller-side capability emitters do)
    // lands identically.
    #[tokio::test]
    async fn the_chain_is_shared_by_every_emitter() {
        let broadcaster = Arc::new(Broadcaster::new());
        let mut sub = broadcaster.subscribe(SubscriptionMode::From(0));
        let agent_sink = turn_emit_pipeline(broadcaster.clone(), false);
        agent_sink.emit(Event {
            kind: "notice".to_owned(),
            text: "from a host emitter".to_owned(),
            ..Event::default()
        });
        let frame = sub.recv().await.expect("frame");
        let w: serde_json::Value = serde_json::from_slice(&frame.data).unwrap();
        assert_eq!(w["kind"], "notice");
        assert_eq!(w["text"], "from a host emitter");
    }

    // One broadcaster per open, never shared (Go: `NewBroadcaster()` per
    // pane — two panes on one hub never read each other's fan-out).
    #[test]
    fn the_pane_kit_mints_one_broadcaster_per_open() {
        let factory = PaneKit::new(false);
        let a = factory.kit();
        let b = factory.kit();
        assert!(!Arc::ptr_eq(&a.broadcaster, &b.broadcaster));
    }
}
