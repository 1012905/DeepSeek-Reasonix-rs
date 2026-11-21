//! The SSE transport (Go `eventstream.go`): each event is one `data:` frame
//! of the JSON wire form, and every frame a client cannot afford to miss also
//! carries an `id:` it can resume from. Keepalives ride every 15 seconds so
//! reverse proxies do not close a quiet stream, and each one carries a real
//! watermark frame a comment would never reach `onmessage` with.
//!
//! Per connection (OPT-002): one task drives the stream with `select!` over
//! frames, the keepalive deadline, and the response channel closing — a
//! client that disconnects tears the task down without orphaning anything.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;

use axum::body::Body;
use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Uri};
use axum::response::Response;

/// The EventSource resume header (http's crate does not name it).
const LAST_EVENT_ID: HeaderName = HeaderName::from_static("last-event-id");
use axum::Router;
use axum::response::IntoResponse;
use axum::routing::get;

use reasonix_session::controller::BroadcasterHandle;

use crate::broadcaster::{Subscription, SubscriptionMode};

/// Go `sseKeepaliveInterval`: how often /events emits a `: ping`. The comment
/// keeps the socket warm under idle-proxy timeouts; the consumer drops it.
pub const SSE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Go `SSEWatermarkInterval`: the transport restates the last numbered frame
/// on the keepalive period, so how long a stalled view stays wrong does not
/// depend on which transport is carrying it.
pub const SSE_WATERMARK_INTERVAL: Duration = SSE_KEEPALIVE_INTERVAL;

/// The replay budget: 8MiB of numbered frames held for resumes
/// (eventstream.go `replayBudget`).
pub const REPLAY_BUDGET: usize = crate::broadcaster::REPLAY_BUDGET;

/// The live queue budget: 4MiB per subscriber before shedding
/// (eventstream.go `queueBudget`).
pub const QUEUE_BUDGET: usize = crate::broadcaster::QUEUE_BUDGET;

/// The stream opening comment, flushed before anything else.
pub const CONNECTED: &[u8] = b": connected\n\n";

/// Renders one SSE frame as a single write (OPT-013): the `id:` line, the
/// data line, and the blank terminator never cross a write boundary, and
/// unnumbered frames get no `id:` so a reconnect never asks to have a stream
/// of deltas replayed at it.
pub fn frame_bytes(seq: i64, data: &[u8]) -> Bytes {
    let mut buf = Vec::with_capacity(data.len() + 32);
    if seq > 0 {
        buf.extend_from_slice(format!("id: {seq}\n").as_bytes());
    }
    buf.extend_from_slice(b"data: ");
    buf.extend_from_slice(data);
    buf.extend_from_slice(b"\n\n");
    Bytes::from(buf)
}

/// The keepalive: a comment plus a real frame (a comment never reaches
/// onmessage; the watermark is how a client learns of a lost last frame
/// within one interval). Go writes the two and flushes once — one write here.
pub fn keepalive_bytes(watermark: i64) -> Bytes {
    let mut buf = Vec::with_capacity(96);
    buf.extend_from_slice(b": ping\n\n");
    buf.extend_from_slice(b"data: {\"kind\":\"stream_watermark\",\"seq\":");
    buf.extend_from_slice(watermark.to_string().as_bytes());
    buf.extend_from_slice(b"}\n\n");
    Bytes::from(buf)
}

/// Go `lastEventID`: the resume point a client carries — the EventSource
/// header it sends unprompted, or the query parameter for clients that cannot
/// set headers. Anything unparsable or negative reads as no cursor.
pub fn last_event_id(headers: &HeaderMap, uri: &Uri) -> i64 {
    let mut raw = headers
        .get(LAST_EVENT_ID)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if raw.is_empty() {
        raw = uri
            .query()
            .and_then(|q| crate::auth::query_param(q, "lastEventId"))
            .unwrap_or_default();
    }
    match raw.trim().parse::<i64>() {
        Ok(seq) if seq >= 0 => seq,
        _ => 0,
    }
}

/// Go `attaching`: whether a request carries no resume cursor. Both
/// transports tell an attach from a resume the same way, so a polled one
/// cannot drift from the stream's answer to the same question.
pub fn attaching(headers: &HeaderMap, uri: &Uri) -> bool {
    if headers.get(LAST_EVENT_ID).is_some_and(|v| !v.is_empty()) {
        return false;
    }
    uri.query()
        .and_then(|q| crate::auth::query_param(q, "lastEventId"))
        .is_none()
}

/// Go `subscribeEvents`' switch: which subscription mode one client's view
/// opens with. `first` is a client that holds nothing yet; `attached` marks a
/// window that wants the watermark prefix.
pub fn subscription_mode(after: i64, first: bool, attached: bool) -> SubscriptionMode {
    match (first, attached) {
        (true, true) => SubscriptionMode::Attached,
        (true, false) => SubscriptionMode::InitialNotices,
        (false, true) => SubscriptionMode::Resume(after),
        (false, false) => SubscriptionMode::From(after),
    }
}

/// The per-runtime event routes (Go routes.go:21-22), mounted under a
/// runtime's prefix by the hub router's dispatch and served unprefixed by
/// routeDefault. With no fan-out published — a host-adopted runtime — the
/// paths answer the same 503 they gave before the mux mounted.
///
/// The controller rides along for the attach half: the prompt a run is
/// stopped on is connection-local state the numbered log never carries, so a
/// client that attaches has to read it from the run itself (Go
/// `pendingPromptSnapshot`).
pub fn events_routes(
    bc: Option<Arc<dyn BroadcasterHandle>>,
    ctrl: Option<Arc<reasonix_session::turns::TurnController>>,
) -> Router {
    match bc {
        Some(bc) => Router::new()
            .route("/events", get(events_handler).fallback(wrong_method))
            .route(
                "/events/replay",
                get(events_replay_handler).fallback(wrong_method),
            )
            .with_state(StreamState { bc, ctrl }),
        None => Router::new()
            .route("/events", get(no_stream).fallback(wrong_method))
            .route("/events/replay", get(no_stream).fallback(wrong_method)),
    }
}

/// The fan-out handle plus the run that answers for its pending prompts.
#[derive(Clone)]
struct StreamState {
    bc: Arc<dyn BroadcasterHandle>,
    ctrl: Option<Arc<reasonix_session::turns::TurnController>>,
}

async fn no_stream() -> Response {
    crate::refuse::refuse_response(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        &crate::refuse::Reason::new(crate::refuse::NO_RUNTIME_OPEN, "no runtime is open"),
    )
}

async fn wrong_method() -> impl IntoResponse {
    crate::refuse::method_not_allowed()
}

/// Go `events`: streams the controller's event flow as SSE until the client
/// disconnects. Subscribe and the pending-prompt read are one handoff under
/// the prompt-emission lock (Go `ReplayPendingPromptsWith`), so a card raised
/// between the two reaches this client either as its own event or in the
/// snapshot the attach answers with.
async fn events_handler(
    State(state): State<StreamState>,
    request: axum::extract::Request<Body>,
) -> Response {
    let after = last_event_id(request.headers(), request.uri());
    let first = attaching(request.headers(), request.uri());
    let mode = subscription_mode(after, first, false);
    let bc = state.bc.clone();
    // The handoff's snapshot is the one this stream opens with: reading it
    // again later would deliver a card twice — once live, once replayed.
    let (subscription, pending) = match &state.ctrl {
        Some(ctrl) => ctrl.prompt_handoff(|| bc.subscribe(mode)),
        None => (bc.subscribe(mode), Default::default()),
    };
    stream_response(subscription, bc, pending)
}

fn stream_response(
    subscription: Subscription,
    bc: Arc<dyn BroadcasterHandle>,
    pending: reasonix_agent::approval::PendingPrompts,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, Infallible>>(16);
    let watermark_source = bc.clone();
    tokio::spawn(async move {
        let mut subscription = subscription;
        if tx.send(Ok(Bytes::from_static(CONNECTED))).await.is_err() {
            return;
        }
        // A card that was pending when this client attached goes out right
        // after the connect comment: the emission it missed was
        // connection-local, so the stream owes the state, not the history.
        for event in pending.events() {
            if let Ok(data) = serde_json::to_vec(&event)
                && tx.send(Ok(frame_bytes(0, &data))).await.is_err()
            {
                return;
            }
        }
        let mut deadline = tokio::time::Instant::now() + SSE_KEEPALIVE_INTERVAL;
        loop {
            tokio::select! {
                frame = subscription.recv() => match frame {
                    None => break,
                    Some(f) => {
                        if tx.send(Ok(frame_bytes(f.seq, &f.data))).await.is_err() {
                            break;
                        }
                    }
                },
                _ = tokio::time::sleep_until(deadline) => {
                    deadline = tokio::time::Instant::now() + SSE_KEEPALIVE_INTERVAL;
                    let mark = watermark_source.watermark();
                    if tx.send(Ok(keepalive_bytes(mark))).await.is_err() {
                        break;
                    }
                }
                _ = tx.closed() => break,
            }
        }
    });
    let mut response = Response::new(Body::from_stream(
        tokio_stream::wrappers::ReceiverStream::new(rx),
    ));
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        axum::http::header::CONNECTION,
        HeaderValue::from_static("keep-alive"),
    );
    response
}

/// Go `eventsReplay`: what a client missed. `complete` false means the log no
/// longer reaches that far back, so each read model has to be re-read from
/// the authority that answers it — a hole looks like a quiet turn, and is
/// said plainly. An attach owes no numbered frames, but it does owe the
/// connection-local state no numbered frame carries: the prompt the run is
/// stopped on goes out as unnumbered frames ahead of the caller's watermark.
async fn events_replay_handler(
    State(state): State<StreamState>,
    request: axum::extract::Request<Body>,
) -> Response {
    let first = attaching(request.headers(), request.uri());
    if first {
        let mut frames: Vec<Bytes> = Vec::new();
        if let Some(ctrl) = &state.ctrl {
            for event in ctrl.pending_prompts().events() {
                if let Ok(data) = serde_json::to_vec(&event) {
                    frames.push(Bytes::from(data));
                }
            }
        }
        return write_replay_frames(&frames, true, state.bc.watermark());
    }
    let after = last_event_id(request.headers(), request.uri());
    let (raw_frames, complete) = state.bc.replay(after);
    write_replay_frames(&raw_frames, complete, state.bc.watermark())
}

/// Go `writeEventsReplayFrames`: `{"frames":[...],"complete":bool,"watermark":N}`
/// with `frames` never null. The frame JSON is embedded raw, exactly as Go's
/// `json.RawMessage` is.
fn write_replay_frames(frames: &[Bytes], complete: bool, watermark: i64) -> Response {
    let mut body = Vec::with_capacity(64 + frames.iter().map(|f| f.len() + 1).sum::<usize>());
    body.extend_from_slice(b"{\"frames\":");
    if frames.is_empty() {
        body.extend_from_slice(b"[]");
    } else {
        body.push(b'[');
        for (i, f) in frames.iter().enumerate() {
            if i > 0 {
                body.push(b',');
            }
            body.extend_from_slice(f);
        }
        body.push(b']');
    }
    body.extend_from_slice(
        format!(r#","complete":{complete},"watermark":{watermark}}}"#).as_bytes(),
    );
    body.push(b'\n');
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let mut response = Response::new(Body::from(bytes::Bytes::from(body)));
    *response.headers_mut() = headers;
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broadcaster::Broadcaster;

    use reasonix_contract::eventwire::event::Event;
    use reasonix_contract::eventwire::tool::Tool;
    use tokio_stream::StreamExt as _StreamExt;

    fn tool_result(id: &str) -> Event {
        Event {
            kind: "tool_result".to_owned(),
            tool: Some(Tool {
                id: id.to_owned(),
                name: "bash".to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        }
    }

    fn kind_of(frame: &crate::broadcaster::Frame) -> String {
        serde_json::from_slice::<serde_json::Value>(&frame.data)
            .ok()
            .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_owned))
            .unwrap_or_default()
    }

    async fn next_frame(ch: &mut Subscription) -> crate::broadcaster::Frame {
        tokio::time::timeout(Duration::from_secs(2), ch.recv())
            .await
            .expect("timed out waiting for a frame")
            .expect("stream closed early")
    }

    // Ported: eventstream_test.go TestOnlyRecoverableFramesAreNumbered.
    #[tokio::test]
    async fn only_recoverable_frames_are_numbered() {
        let b = Broadcaster::new();
        let mut ch = b.subscribe(SubscriptionMode::From(0));
        b.emit(Event {
            kind: "text".to_owned(),
            text: "streaming".to_owned(),
            ..Event::default()
        });
        b.emit(tool_result("t1"));
        b.emit(Event {
            kind: "reasoning".to_owned(),
            text: "thinking".to_owned(),
            ..Event::default()
        });
        b.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        let want: [(&str, i64); 4] = [
            ("text", 0),
            ("tool_result", 1),
            ("reasoning", 0),
            ("turn_done", 2),
        ];
        for (kind, seq) in want {
            let f = next_frame(&mut ch).await;
            assert_eq!(kind_of(&f), kind);
            assert_eq!(f.seq, seq, "{kind} numbering");
        }
        assert_eq!(b.watermark(), 2);
    }

    // Ported: eventstream_test.go TestSubscribeFromReplaysTheGap.
    #[tokio::test]
    async fn subscribe_from_replays_the_gap() {
        let b = Broadcaster::new();
        for id in ["t1", "t2", "t3"] {
            b.emit(tool_result(id));
        }
        // A client that saw frame 1 comes back.
        let mut ch = b.subscribe(SubscriptionMode::From(1));
        b.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        for want in [r#""id":"t2""#, r#""id":"t3""#, r#""kind":"turn_done""#] {
            let f = next_frame(&mut ch).await;
            let s = String::from_utf8_lossy(&f.data);
            assert!(s.contains(want), "frame {s} must contain {want}");
        }
    }

    // Ported: eventstream_test.go TestSubscribeFromCurrentReplaysNothing.
    #[tokio::test]
    async fn subscribe_from_current_replays_nothing() {
        let b = Broadcaster::new();
        b.emit(tool_result("t1"));
        let mut ch = b.subscribe(SubscriptionMode::From(b.watermark()));
        b.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        let f = next_frame(&mut ch).await;
        assert_eq!(kind_of(&f), "turn_done", "want only the live turn_done");
    }

    // Ported: eventstream_test.go
    // TestSubscribeInitialNoticesSkipsOtherHistory.
    #[tokio::test]
    async fn initial_notices_skip_other_history() {
        let b = Broadcaster::new();
        b.emit(tool_result("before"));
        b.emit(Event {
            kind: "notice".to_owned(),
            text: "startup one".to_owned(),
            ..Event::default()
        });
        b.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        b.emit(Event {
            kind: "notice".to_owned(),
            text: "startup two".to_owned(),
            ..Event::default()
        });
        let mut ch = b.subscribe(SubscriptionMode::InitialNotices);
        let first = next_frame(&mut ch).await;
        let w: serde_json::Value = serde_json::from_slice(&first.data).unwrap();
        assert_eq!(first.seq, 0, "initial replay must be unnumbered");
        assert_eq!(
            w.get("seq").and_then(|v| v.as_i64()).unwrap_or(0),
            0,
            "the copy itself must carry no seq"
        );
        assert_eq!(w["kind"], "notice");
        assert!(String::from_utf8_lossy(&first.data).contains("startup one"));
        let second = next_frame(&mut ch).await;
        assert_eq!(second.seq, 0);
        assert!(String::from_utf8_lossy(&second.data).contains("startup two"));
        b.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        let next = next_frame(&mut ch).await;
        assert_eq!(kind_of(&next), "turn_done");
        assert_eq!(next.seq, 5);
    }

    // Ported: eventstream_test.go
    // TestSubscribeInitialNoticesDoesNotCrossSessionReset.
    #[tokio::test]
    async fn initial_notices_do_not_cross_a_session_reset() {
        let b = Broadcaster::new();
        b.emit(Event {
            kind: "notice".to_owned(),
            text: "old session".to_owned(),
            ..Event::default()
        });
        b.reset_session();
        b.emit(Event {
            kind: "notice".to_owned(),
            text: "current session".to_owned(),
            ..Event::default()
        });
        let mut ch = b.subscribe(SubscriptionMode::InitialNotices);
        let first = next_frame(&mut ch).await;
        assert!(String::from_utf8_lossy(&first.data).contains("current session"));
        b.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        let next = next_frame(&mut ch).await;
        assert_eq!(kind_of(&next), "turn_done");
    }

    // Ported: eventstream_test.go
    // TestSubscribeInitialNoticesOnlyReplaysOncePerSession.
    #[tokio::test]
    async fn initial_notices_replay_only_once_per_session() {
        let b = Broadcaster::new();
        b.emit(Event {
            kind: "notice".to_owned(),
            text: "startup".to_owned(),
            ..Event::default()
        });
        let mut first = b.subscribe(SubscriptionMode::InitialNotices);
        let got = next_frame(&mut first).await;
        assert_eq!(kind_of(&got), "notice");
        drop(first);

        b.emit(Event {
            kind: "notice".to_owned(),
            text: "later".to_owned(),
            ..Event::default()
        });
        let mut second = b.subscribe(SubscriptionMode::InitialNotices);
        b.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        let got = next_frame(&mut second).await;
        assert_eq!(
            kind_of(&got),
            "turn_done",
            "second window replayed old notice"
        );
        drop(second);

        b.reset_session();
        b.emit(Event {
            kind: "notice".to_owned(),
            text: "new session".to_owned(),
            ..Event::default()
        });
        let mut third = b.subscribe(SubscriptionMode::InitialNotices);
        let got = next_frame(&mut third).await;
        assert_eq!(kind_of(&got), "notice");
        assert!(String::from_utf8_lossy(&got.data).contains("new session"));
    }

    // Ported: eventstream_test.go
    // TestSubscribeFromTooFarBackAnnouncesTheGap.
    #[tokio::test]
    async fn subscribing_too_far_back_announces_the_gap() {
        let b = Broadcaster::new();
        for _ in 0..3 {
            b.emit(tool_result("t"));
        }
        b.reset_session(); // what the budget does to the oldest frames under load
        let mut ch = b.subscribe(SubscriptionMode::From(1));
        let f = next_frame(&mut ch).await;
        assert_eq!(kind_of(&f), "stream_gap", "first frame must be the gap");
        assert!(
            f.seq > 0,
            "a gap must say where the trustworthy stream starts"
        );
    }

    // Ported: eventstream_test.go TestReplayReportsWhatItCannotReach.
    #[tokio::test]
    async fn replay_reports_what_it_cannot_reach() {
        let b = Broadcaster::new();
        for _ in 0..3 {
            b.emit(tool_result("t"));
        }
        let (frames, complete) = b.replay(1);
        assert!(complete);
        assert_eq!(frames.len(), 2);
        let (_, complete) = b.replay(b.watermark());
        assert!(complete, "a current client has missed nothing");
        b.reset_session();
        let (_, complete) = b.replay(1);
        assert!(
            !complete,
            "an evicted range must report itself unrecoverable, not empty-and-fine"
        );
    }

    // Ported: eventstream_test.go
    // TestResetSessionRetiresReplayButKeepsNumbering.
    #[tokio::test]
    async fn reset_session_retires_replay_but_keeps_numbering() {
        let b = Broadcaster::new();
        b.emit(tool_result("old"));
        let before = b.watermark();
        b.reset_session();
        b.emit(tool_result("new"));
        assert!(
            b.watermark() > before,
            "watermark went backwards across a switch"
        );
        let (_, complete) = b.replay(before - 1);
        assert!(
            !complete,
            "frames from the retired session must not be replayable"
        );
    }

    // Ported: eventstream_test.go
    // TestSubscribeAttachedStatesWhereItAttachedBeforeAnyLaterFrame.
    #[tokio::test]
    async fn attached_subscription_states_where_it_attached() {
        let b = Broadcaster::new();
        for _ in 0..3 {
            b.emit(tool_result("t"));
        }
        let mut ch = b.subscribe(SubscriptionMode::Attached);
        b.emit(Event {
            kind: "turn_done".to_owned(),
            ..Event::default()
        });
        let first = next_frame(&mut ch).await;
        let mark: serde_json::Value = serde_json::from_slice(&first.data).unwrap();
        assert_eq!(mark["kind"], "stream_watermark");
        assert_eq!(mark["seq"], 3, "watermark must name the attach point");
        assert_eq!(first.seq, 0, "the watermark itself is unnumbered");
        let next = next_frame(&mut ch).await;
        assert_eq!(kind_of(&next), "turn_done");
        assert_eq!(next.seq, 4, "live frames number on after the watermark");
    }

    // Ported: eventstream_test.go
    // TestSubscribeResumeFromZeroReplaysTheWholeLog.
    #[tokio::test]
    async fn resume_from_zero_replays_the_whole_log() {
        let b = Broadcaster::new();
        for _ in 0..3 {
            b.emit(tool_result("t"));
        }
        let mut ch = b.subscribe(SubscriptionMode::Resume(0));
        for want in 1..=3i64 {
            let got = next_frame(&mut ch).await;
            assert_eq!(got.seq, want, "replayed frame ordering");
        }
    }

    #[test]
    fn frame_renders_id_only_for_numbered_frames() {
        assert_eq!(
            frame_bytes(0, br#"{"kind":"text"}"#).as_ref(),
            &b"data: {\"kind\":\"text\"}\n\n"[..]
        );
        assert_eq!(
            frame_bytes(4, br#"{"kind":"turn_done"}"#).as_ref(),
            &b"id: 4\ndata: {\"kind\":\"turn_done\"}\n\n"[..]
        );
    }

    #[test]
    fn keepalive_carries_comment_and_watermark_in_one_write() {
        let bytes = keepalive_bytes(7);
        assert_eq!(
            bytes.as_ref(),
            &b": ping\n\ndata: {\"kind\":\"stream_watermark\",\"seq\":7}\n\n"[..]
        );
    }

    #[test]
    fn last_event_id_reads_header_then_query_and_never_negatives() {
        let mut headers = HeaderMap::new();
        headers.insert(LAST_EVENT_ID, HeaderValue::from_static("12"));
        let uri: Uri = "/events".parse().unwrap();
        assert_eq!(last_event_id(&headers, &uri), 12);
        headers.clear();
        let uri: Uri = "/events?lastEventId=9".parse().unwrap();
        assert_eq!(last_event_id(&headers, &uri), 9);
        let uri: Uri = "/events?lastEventId=-3".parse().unwrap();
        assert_eq!(last_event_id(&headers, &uri), 0);
        let uri: Uri = "/events?lastEventId=abc".parse().unwrap();
        assert_eq!(last_event_id(&headers, &uri), 0);
        let uri: Uri = "/events".parse().unwrap();
        assert_eq!(last_event_id(&headers, &uri), 0);
    }

    #[test]
    fn attaching_splits_resume_from_attach() {
        let mut headers = HeaderMap::new();
        let uri: Uri = "/events".parse().unwrap();
        assert!(attaching(&headers, &uri));
        headers.insert(LAST_EVENT_ID, HeaderValue::from_static("3"));
        assert!(!attaching(&headers, &uri));
        headers.clear();
        let uri: Uri = "/events?lastEventId=3".parse().unwrap();
        assert!(!attaching(&headers, &uri));
    }

    #[test]
    fn subscription_mode_matches_the_go_switch() {
        assert_eq!(subscription_mode(5, true, true), SubscriptionMode::Attached);
        assert_eq!(
            subscription_mode(5, true, false),
            SubscriptionMode::InitialNotices
        );
        assert_eq!(
            subscription_mode(5, false, true),
            SubscriptionMode::Resume(5)
        );
        assert_eq!(
            subscription_mode(5, false, false),
            SubscriptionMode::From(5)
        );
    }

    #[tokio::test]
    async fn replay_endpoint_answers_frames_complete_watermark() {
        let b = Arc::new(Broadcaster::new());
        b.emit(tool_result("t1"));
        b.emit(tool_result("t2"));
        let router = events_routes(Some(b.clone()), None);
        let request = axum::extract::Request::builder()
            .uri("/events/replay?lastEventId=1")
            .body(Body::empty())
            .unwrap();
        let response = oneshot(router, request).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["complete"], true);
        assert_eq!(json["watermark"], 2);
        let frames = json["frames"].as_array().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["kind"], "tool_result");
        assert_eq!(frames[0]["tool"]["id"], "t2");
    }

    #[tokio::test]
    async fn replay_endpoint_answers_empty_array_never_null_for_a_current_client() {
        let b = Arc::new(Broadcaster::new());
        let router = events_routes(Some(b.clone()), None);
        let request = axum::extract::Request::builder()
            .uri("/events/replay")
            .body(Body::empty())
            .unwrap();
        let response = oneshot(router, request).await;
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("\"frames\":[]"));
    }

    /// Drives a router to one response (tower's ServiceExt::oneshot, a
    /// dev-only convenience).
    async fn oneshot(router: Router, request: axum::extract::Request<Body>) -> Response {
        use tower::ServiceExt as _ServiceExt;
        router
            .oneshot(request)
            .await
            .expect("router call cannot fail")
    }

    #[tokio::test]
    async fn events_stream_opens_with_connected_then_numbers_frames() {
        let b = Arc::new(Broadcaster::new());
        let router = events_routes(Some(b.clone()), None);
        let request = axum::extract::Request::builder()
            .uri("/events")
            .body(Body::empty())
            .unwrap();
        let response = oneshot(router, request).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.headers().get(CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/event-stream"))
        );
        assert_eq!(
            response.headers().get(CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-cache"))
        );
        let mut stream = response.into_body().into_data_stream();
        let first = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("connect frame timed out")
            .expect("stream ended early")
            .expect("frame bytes");
        assert_eq!(first, Bytes::from_static(b": connected\n\n"));
        b.emit(Event {
            kind: "tool_result".to_owned(),
            tool: Some(Tool {
                id: "t1".to_owned(),
                name: "bash".to_owned(),
                ..Tool::default()
            }),
            ..Event::default()
        });
        let frame = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("frame timed out")
            .expect("stream ended early")
            .expect("frame bytes");
        assert!(frame.starts_with(b"id: 1\ndata: "));
        assert!(frame.ends_with(b"\n\n"));
    }

    /// A controller with an interactive approval surface and nothing else: a
    /// dead provider is never reached, because the asks under test come from
    /// the test's own question rather than a turn.
    fn asking_controller() -> Arc<reasonix_session::turns::TurnController> {
        use reasonix_agent::approval::{ApprovalConfig, mode};
        use reasonix_session::turns::{TurnController, TurnControllerParts};

        let mut cfg = reasonix_provider::config::ClientConfig::new("t", "http://127.0.0.1:1", "m");
        cfg.api_key = "k".to_owned();
        TurnController::new(TurnControllerParts {
            config: reasonix_agent::config::AgentConfig::default(),
            client: reasonix_provider::config::build_client(&cfg).expect("client builds"),
            registry: Arc::new(reasonix_tools::Registry::new()),
            sink: Arc::new(reasonix_agent::sink::Discard),
            workspace_root: ".".to_owned(),
            session_dir: String::new(),
            label: "t".to_owned(),
            approval: Some(ApprovalConfig {
                mode: mode::ASK,
                timeout: Duration::from_secs(5),
            }),
        })
    }

    fn one_question() -> Vec<reasonix_contract::eventwire::ask::AskQuestion> {
        use reasonix_contract::eventwire::ask::{AskOption, AskQuestion};
        vec![AskQuestion {
            id: "q1".to_owned(),
            header: "Library".to_owned(),
            prompt: "which".to_owned(),
            options: vec![
                AskOption {
                    label: "A".to_owned(),
                    description: String::new(),
                },
                AskOption {
                    label: "B".to_owned(),
                    description: String::new(),
                },
            ],
            ..AskQuestion::default()
        }]
    }

    /// The attach replay carries the card a reconnecting client must
    /// re-render, as an unnumbered frame, and the answer lands against the id
    /// it named (Go `pendingPromptSnapshot` + `ReplayPendingPromptsWith`).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_attach_replays_the_open_question_and_its_answer_lands() {
        let ctrl = asking_controller();
        // The ask blocks its caller, so it waits on its own thread.
        let ask_ctrl = ctrl.clone();
        let asked = std::thread::spawn(move || ask_ctrl.ask(&one_question()));
        let id = loop {
            let pending = ctrl.pending_prompts();
            if let Some(ask) = pending.asks.first() {
                break ask.id.clone();
            }
            std::thread::sleep(Duration::from_millis(2));
        };

        let b = Arc::new(Broadcaster::new());
        let router = events_routes(Some(b.clone()), Some(ctrl.clone()));
        let request = axum::extract::Request::builder()
            .uri("/events/replay")
            .body(Body::empty())
            .unwrap();
        let response = oneshot(router, request).await;
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["complete"], true);
        let frames = json["frames"].as_array().unwrap();
        assert_eq!(frames.len(), 1, "the attach owes the open card: {json}");
        assert_eq!(frames[0]["kind"], "ask_request");
        assert_eq!(frames[0]["ask"]["id"], id);
        assert_eq!(frames[0]["ask"]["questions"][0]["header"], "Library");

        // The answer lands against the id the replay named.
        assert!(ctrl.answer(
            &id,
            vec![reasonix_agent::approval::AskAnswer {
                id: "q1".to_owned(),
                selected: vec!["B".to_owned()],
            }]
        ));
        let result = asked.join().expect("ask thread").expect("ask ran");
        assert!(!result.timed_out);
        drop(ctrl);
    }

    /// A headless runtime owes no card: the attach answers an empty frame
    /// list rather than a prompt nobody can answer.
    #[tokio::test]
    async fn an_attach_without_an_approval_surface_answers_no_frames() {
        let b = Arc::new(Broadcaster::new());
        let router = events_routes(Some(b), None);
        let request = axum::extract::Request::builder()
            .uri("/events/replay")
            .body(Body::empty())
            .unwrap();
        let response = oneshot(router, request).await;
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["frames"].as_array().unwrap().len(), 0);
        assert_eq!(json["complete"], true);
    }
}
