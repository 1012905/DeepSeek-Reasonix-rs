//! Reconnecting wrapper behavior tests (MCP-013, contract §7).
//!
//! A scripted transport double drives connection death, replacement, shared
//! dial, rebind, and teardown; no subprocess or third-party server is
//! involved.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reasonix_ext::mcp::{
    Dialer, Era, OwnedTransport, ReconnectingTransport, Transport, TransportError,
};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct DoubleState {
    calls: Vec<(String, Option<Value>)>,
    notifications: Vec<(String, Option<Value>)>,
    closed: bool,
}

/// Deterministic transport double: replies are popped from a scripted list,
/// call/notify traffic and the close flag are recorded behind shared state.
#[derive(Clone)]
struct Double {
    era: Era,
    state: Arc<Mutex<DoubleState>>,
    replies: Arc<Mutex<Vec<Result<Value, TransportError>>>>,
}

impl Double {
    fn new(era: Era, replies: Vec<Result<Value, TransportError>>) -> Self {
        Self {
            era,
            state: Arc::new(Mutex::new(DoubleState::default())),
            replies: Arc::new(Mutex::new(replies)),
        }
    }

    fn calls(&self) -> Vec<(String, Option<Value>)> {
        self.state.lock().expect("double state").calls.clone()
    }

    fn notifications(&self) -> Vec<(String, Option<Value>)> {
        self.state
            .lock()
            .expect("double state")
            .notifications
            .clone()
    }

    fn closed(&self) -> bool {
        self.state.lock().expect("double state").closed
    }
}

impl Transport for Double {
    fn call(&self, method: &str, params: Option<&Value>) -> Result<Value, TransportError> {
        self.state
            .lock()
            .expect("double state")
            .calls
            .push((method.to_string(), params.cloned()));
        let mut replies = self.replies.lock().expect("double replies");
        replies.pop().unwrap_or(Err(TransportError::NoReply))
    }

    fn notify(&self, method: &str, params: Option<&Value>) -> Result<(), TransportError> {
        self.state
            .lock()
            .expect("double state")
            .notifications
            .push((method.to_string(), params.cloned()));
        Ok(())
    }

    fn era(&self) -> &Era {
        &self.era
    }
}

impl OwnedTransport for Double {
    fn close(&self) {
        self.state.lock().expect("double state").closed = true;
    }
}

/// Scripted dialer: counts dials, hands out queued doubles, can inject a
/// startup failure or a delay to model a slow in-flight dial.
struct TestDialer {
    dials: AtomicUsize,
    queue: Mutex<Vec<Double>>,
    delay: Mutex<Option<Duration>>,
    fail: Mutex<Option<TransportError>>,
    produced: Mutex<Vec<Double>>,
}

impl TestDialer {
    fn new(queue: Vec<Double>) -> Arc<Self> {
        Arc::new(Self {
            dials: AtomicUsize::new(0),
            queue: Mutex::new(queue),
            delay: Mutex::new(None),
            fail: Mutex::new(None),
            produced: Mutex::new(Vec::new()),
        })
    }

    fn dial_count(&self) -> usize {
        self.dials.load(Ordering::SeqCst)
    }

    fn set_delay(&self, delay: Duration) {
        *self.delay.lock().expect("dialer delay") = Some(delay);
    }

    fn set_failure(&self, err: TransportError) {
        *self.fail.lock().expect("dialer failure") = Some(err);
    }

    fn produced(&self) -> Vec<Double> {
        self.produced.lock().expect("dialer produced").clone()
    }
}

impl Dialer for TestDialer {
    fn dial(
        &self,
        _budget: Duration,
    ) -> Result<Box<dyn OwnedTransport + Send + Sync>, TransportError> {
        self.dials.fetch_add(1, Ordering::SeqCst);
        if let Some(err) = self.fail.lock().expect("dialer failure").clone() {
            return Err(err);
        }
        if let Some(delay) = *self.delay.lock().expect("dialer delay") {
            std::thread::sleep(delay);
        }
        let next = self
            .queue
            .lock()
            .expect("dialer queue")
            .pop()
            .expect("dialer script exhausted");
        self.produced
            .lock()
            .expect("dialer produced")
            .push(next.clone());
        Ok(Box::new(next))
    }
}

fn wait_for(predicate: impl Fn() -> bool, label: &str) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {label}");
}

#[test]
fn call_replays_once_after_gone_then_succeeds_on_replacement() {
    let d1 = Double::new(Era::legacy(), vec![Err(TransportError::Gone)]);
    let d2 = Double::new(Era::modern("2025-06-18"), vec![Ok(json!({"pong": true}))]);
    let dialer = TestDialer::new(vec![d2.clone()]);
    let wrapper = ReconnectingTransport::new(Arc::new(d1.clone()), dialer.clone());

    let reply = wrapper.call("ping", None).expect("replacement answers");
    assert_eq!(reply, json!({"pong": true}));
    assert_eq!(dialer.dial_count(), 1, "one replacement dial");
    assert!(d1.closed(), "stale connection closed after replacement");
    assert_eq!(d2.calls().len(), 1, "replacement carries the replayed call");
    assert_eq!(wrapper.era(), Some(d2.era.clone()));
}

#[test]
fn second_death_on_replacement_is_not_retried() {
    let d1 = Double::new(Era::legacy(), vec![Err(TransportError::Gone)]);
    let d2 = Double::new(Era::legacy(), vec![Err(TransportError::Gone)]);
    let dialer = TestDialer::new(vec![d2.clone()]);
    let wrapper = ReconnectingTransport::new(Arc::new(d1.clone()), dialer.clone());

    let err = wrapper
        .call("ping", None)
        .expect_err("replacement also gone");
    assert_eq!(err, TransportError::Gone);
    assert_eq!(dialer.dial_count(), 1, "no second dial for one call");
    assert_eq!(d2.calls().len(), 1, "replacement tried once, not looped");
}

#[test]
fn concurrent_callers_share_one_dial() {
    const CALLERS: usize = 8;
    let d1 = Double::new(Era::legacy(), vec![Err(TransportError::Gone); CALLERS]);
    let d2 = Double::new(Era::legacy(), vec![Ok(json!({"ok": 1})); CALLERS]);
    let dialer = TestDialer::new(vec![d2.clone()]);
    let wrapper = Arc::new(ReconnectingTransport::new(
        Arc::new(d1.clone()),
        dialer.clone(),
    ));

    let mut handles = Vec::new();
    for _ in 0..CALLERS {
        let wrapper = wrapper.clone();
        handles.push(std::thread::spawn(move || wrapper.call("tools/list", None)));
    }
    for handle in handles {
        assert!(handle.join().expect("caller joined").is_ok());
    }

    assert_eq!(dialer.dial_count(), 1, "concurrent callers share one dial");
    assert_eq!(
        d2.calls().len(),
        CALLERS,
        "every caller reaches the replacement"
    );
}

#[test]
fn notify_is_forwarded_without_dialing() {
    let d1 = Double::new(Era::legacy(), vec![]);
    let dialer = TestDialer::new(vec![]);
    let wrapper = ReconnectingTransport::new(Arc::new(d1.clone()), dialer.clone());

    wrapper
        .notify("notifications/initialized", None)
        .expect("notify forwarded");
    assert_eq!(
        dialer.dial_count(),
        0,
        "a fire-and-forget never spawns a dial"
    );
    assert_eq!(d1.notifications().len(), 1);
}

#[test]
fn restart_swaps_to_fresh_double_and_closes_the_old() {
    let d1 = Double::new(Era::legacy(), vec![]);
    let d2 = Double::new(Era::modern("2025-06-18"), vec![Ok(json!(1))]);
    let dialer = TestDialer::new(vec![d2.clone()]);
    let wrapper = ReconnectingTransport::new(Arc::new(d1.clone()), dialer.clone());

    wrapper.restart().expect("restart");
    assert!(
        d1.closed(),
        "old connection shut down before the fresh one ran"
    );
    assert_eq!(dialer.dial_count(), 1);
    assert_eq!(wrapper.era(), Some(d2.era.clone()));

    let reply = wrapper
        .call("ping", None)
        .expect("fresh connection answers");
    assert_eq!(reply, json!(1));
}

#[test]
fn restart_dial_failure_closes_old_and_leaves_no_live_connection() {
    let d1 = Double::new(Era::legacy(), vec![]);
    let dialer = TestDialer::new(vec![]);
    dialer.set_failure(TransportError::Io("spawn failed".into()));
    let wrapper = ReconnectingTransport::new(Arc::new(d1.clone()), dialer.clone());

    let err = wrapper.restart().expect_err("dial failure propagates");
    assert!(matches!(err, TransportError::Io(_)));
    assert!(
        d1.closed(),
        "old connection closed despite the failed restart"
    );
    assert!(
        wrapper.call("ping", None).is_err(),
        "no live connection left"
    );
}

#[test]
fn close_drains_the_inflight_dial_within_budget() {
    let d1 = Double::new(Era::legacy(), vec![Err(TransportError::Gone)]);
    let d2 = Double::new(Era::legacy(), vec![Ok(json!(1))]);
    let dialer = TestDialer::new(vec![d2.clone()]);
    dialer.set_delay(Duration::from_millis(150));
    let wrapper = Arc::new(ReconnectingTransport::new(
        Arc::new(d1.clone()),
        dialer.clone(),
    ));

    let leader = std::thread::spawn({
        let wrapper = wrapper.clone();
        move || wrapper.call("ping", None)
    });
    std::thread::sleep(Duration::from_millis(30));

    let started = Instant::now();
    wrapper.close();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "close waited beyond the dial budget"
    );

    wait_for(|| dialer.produced().len() == 1, "late dial to settle");
    assert!(
        dialer.produced()[0].closed(),
        "a dial arriving after close closes its own connection"
    );
    assert!(leader.join().expect("leader joined").is_err());
}

#[test]
fn late_dial_arriving_after_close_self_closes() {
    let d1 = Double::new(Era::legacy(), vec![Err(TransportError::Gone)]);
    let d2 = Double::new(Era::legacy(), vec![Ok(json!(1))]);
    let dialer = TestDialer::new(vec![d2.clone()]);
    dialer.set_delay(Duration::from_millis(250));
    let wrapper = Arc::new(ReconnectingTransport::with_budgets(
        Arc::new(d1.clone()),
        dialer.clone(),
        None,
        Some(Duration::from_millis(50)),
    ));

    let leader = std::thread::spawn({
        let wrapper = wrapper.clone();
        move || wrapper.call("ping", None)
    });
    std::thread::sleep(Duration::from_millis(30));

    let started = Instant::now();
    wrapper.close();
    assert!(
        started.elapsed() < Duration::from_millis(150),
        "close did not wait out the in-flight dial"
    );

    wait_for(|| dialer.produced().len() == 1, "late dial to settle");
    assert!(dialer.produced()[0].closed(), "late connection self-closes");
    assert!(leader.join().expect("leader joined").is_err());
}

#[test]
fn rebind_callback_receives_every_replacement_connection() {
    let d1 = Double::new(Era::legacy(), vec![Err(TransportError::Gone)]);
    let d2 = Double::new(Era::modern("2025-06-18"), vec![Ok(json!(1))]);
    let d3 = Double::new(Era::legacy(), vec![Ok(json!(2))]);
    let dialer = TestDialer::new(vec![d3.clone(), d2.clone()]);
    let wrapper = ReconnectingTransport::new(Arc::new(d1.clone()), dialer.clone());

    let seen: Arc<Mutex<Vec<Era>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = seen.clone();
    wrapper.register_rebind(Box::new(move |connection: &dyn Transport| {
        recorded
            .lock()
            .expect("rebind record")
            .push(connection.era().clone());
    }));

    wrapper.call("ping", None).expect("replacement answers");
    wrapper.restart().expect("restart");

    let seen = seen.lock().expect("rebind record").clone();
    assert_eq!(seen, vec![d2.era.clone(), d3.era.clone()]);
}

#[test]
fn era_follows_the_active_connection_across_replacement() {
    let d1 = Double::new(Era::legacy(), vec![Err(TransportError::Gone)]);
    let d2 = Double::new(Era::modern("2025-06-18"), vec![Ok(json!(1))]);
    let d3 = Double::new(Era::legacy(), vec![Ok(json!(2))]);
    let dialer = TestDialer::new(vec![d3.clone(), d2.clone()]);
    let wrapper = ReconnectingTransport::new(Arc::new(d1.clone()), dialer.clone());

    wrapper.call("ping", None).expect("replacement answers");
    assert_eq!(wrapper.era(), Some(d2.era.clone()));

    wrapper.restart().expect("restart");
    assert_eq!(wrapper.era(), Some(d3.era.clone()));
}
