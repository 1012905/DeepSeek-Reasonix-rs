//! The reconnecting wrapper: the stable handle consumers hold while the
//! connection underneath rolls (docs/contracts/mcp.md §7; oracle
//! `internal/ext/plugin/transport_reconnect.go`).
//!
//! Only a connection that died before the request was written reports
//! [`TransportError::Gone`]; that death is safe to replay on a replacement. A
//! death mid-request is deliberately not marked — the server may already have
//! run the call — so the wrapper never retries it.

use alloc::boxed::Box;
use alloc::vec::Vec;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::lifecycle::DEFAULT_START_TIMEOUT_MS;
use super::seam::{Dialer, OwnedTransport, Transport, TransportError};
use super::version::Era;

/// Bound on one replacement dial when no caller budget is supplied (ms).
pub const DEFAULT_DIAL_BUDGET_MS: u64 = DEFAULT_START_TIMEOUT_MS;
/// Teardown waits this long for an in-flight dial; a dial still late arrives
/// afterwards and closes its own connection (ms).
pub const CLOSE_WAIT_BUDGET_MS: u64 = 5_000;

type Conn = Arc<dyn OwnedTransport + Send + Sync>;
type RebindCallback = Box<dyn Fn(&dyn Transport) + Send + Sync>;

/// Single-flight token for the dial currently in progress. Waiters block on
/// the condvar; the leader publishes the result under the wrapper lock.
struct Dialing {
    done: Mutex<bool>,
    cond: Condvar,
}

struct Inner {
    active: Option<Conn>,
    dialing: Option<Arc<Dialing>>,
    closed: bool,
}

/// Stable handle over a rolling connection. Callers hold this; the connection
/// it points at can be replaced wholesale underneath them.
pub struct ReconnectingTransport {
    inner: Mutex<Inner>,
    dialer: Arc<dyn Dialer + Send + Sync>,
    budget: Duration,
    close_wait: Duration,
    rebind: Mutex<Vec<RebindCallback>>,
    era: Mutex<Option<Era>>,
}

impl ReconnectingTransport {
    /// Wraps the active connection with the default dial and teardown budgets.
    pub fn new(active: Conn, dialer: Arc<dyn Dialer + Send + Sync>) -> Self {
        Self::with_budgets(active, dialer, None, None)
    }

    /// Test/overload entry: separate dial and close-wait budgets so a test
    /// can shrink the teardown window without sleeping against the default 5s.
    pub fn with_budgets(
        active: Conn,
        dialer: Arc<dyn Dialer + Send + Sync>,
        budget: Option<Duration>,
        close_wait: Option<Duration>,
    ) -> Self {
        let era = active.era().clone();
        Self {
            inner: Mutex::new(Inner {
                active: Some(active),
                dialing: None,
                closed: false,
            }),
            dialer,
            budget: budget.unwrap_or(Duration::from_millis(DEFAULT_DIAL_BUDGET_MS)),
            close_wait: close_wait.unwrap_or(Duration::from_millis(CLOSE_WAIT_BUDGET_MS)),
            rebind: Mutex::new(Vec::new()),
            era: Mutex::new(Some(era)),
        }
    }

    /// Sends a request once; on a connection that was already gone it dials a
    /// replacement and dispatches again. Exactly one replacement per call: a
    /// second death is returned as-is, so a server dying on every spawn costs
    /// one attempt per call rather than a loop.
    pub fn call(&self, method: &str, params: Option<&Value>) -> Result<Value, TransportError> {
        let active = self.current()?;
        let first = active.call(method, params);
        let Err(TransportError::Gone) = first else {
            return first;
        };
        let next = self.renew(active)?;
        next.call(method, params)
    }

    /// Forwards a notification without reconnecting: a lost fire-and-forget
    /// message is not worth a spawn, and the next call reconnects anyway.
    pub fn notify(&self, method: &str, params: Option<&Value>) -> Result<(), TransportError> {
        let active = self.current()?;
        active.notify(method, params)
    }

    /// The era negotiated on the active connection, if one is attached.
    pub fn era(&self) -> Option<Era> {
        self.era.lock().expect("era mutex").clone()
    }

    /// Replaces the connection with a fresh one. The old connection shuts down
    /// first, so a server holding a port or a file lock never meets its own
    /// predecessor; the dial is bounded by the startup budget.
    pub fn restart(&self) -> Result<(), TransportError> {
        // Drain any dial already in flight before swapping; a late settler
        // publishes against the closed-or-swapped state below.
        loop {
            let guard = self.inner.lock().expect("inner mutex");
            if guard.closed {
                return Err(closed_error());
            }
            match &guard.dialing {
                Some(token) => {
                    let token = token.clone();
                    drop(guard);
                    wait_settled(&token);
                }
                None => break,
            }
        }

        let token = Arc::new(Dialing {
            done: Mutex::new(false),
            cond: Condvar::new(),
        });
        let stale = {
            let mut guard = self.inner.lock().expect("inner mutex");
            if guard.closed {
                return Err(closed_error());
            }
            let stale = guard.active.take();
            guard.dialing = Some(token.clone());
            stale
        };
        if let Some(old) = stale {
            old.close();
        }

        let next: Result<Conn, TransportError> = self.dialer.dial(self.budget).map(Arc::from);
        let mut guard = self.inner.lock().expect("inner mutex");
        guard.dialing = None;
        match next {
            Ok(conn) => {
                if guard.closed {
                    drop(guard);
                    conn.close();
                    signal(&token);
                    return Err(closed_error());
                }
                guard.active = Some(conn.clone());
                *self.era.lock().expect("era mutex") = Some(conn.era().clone());
                drop(guard);
                self.run_rebind(&conn);
            }
            Err(err) => {
                drop(guard);
                signal(&token);
                return Err(err);
            }
        }
        signal(&token);
        Ok(())
    }

    /// Records a rebind callback kept at the wrapper. Each replacement
    /// connection is handed to every registered callback, so progress tokens
    /// and notification handlers can be re-bound instead of re-registered.
    pub fn register_rebind(&self, callback: RebindCallback) {
        self.rebind.lock().expect("rebind mutex").push(callback);
    }

    /// Closes the active connection and waits, up to the close budget, for an
    /// in-flight dial. A dial that settles after teardown closes its own
    /// connection.
    pub fn close(&self) {
        let (active, dialing) = {
            let mut guard = self.inner.lock().expect("inner mutex");
            guard.closed = true;
            (guard.active.take(), guard.dialing.clone())
        };
        if let Some(old) = active {
            old.close();
        }
        let Some(token) = dialing else {
            return;
        };
        let deadline = Instant::now() + self.close_wait;
        let mut done = token.done.lock().expect("dialing mutex");
        while !*done {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let (guard, result) = token
                .cond
                .wait_timeout(done, remaining)
                .expect("dialing condvar");
            if result.timed_out() {
                break;
            }
            done = guard;
        }
    }

    fn current(&self) -> Result<Conn, TransportError> {
        let guard = self.inner.lock().expect("inner mutex");
        match &guard.active {
            Some(conn) => Ok(conn.clone()),
            None => Err(closed_error()),
        }
    }

    /// Replaces the stale connection. Concurrent callers that all found the
    /// same dead connection share one dial; a caller that walks away while
    /// waiting leaves the dial running for whoever comes behind.
    fn renew(&self, stale: Conn) -> Result<Conn, TransportError> {
        loop {
            let mut guard = self.inner.lock().expect("inner mutex");
            if guard.closed {
                return Err(closed_error());
            }
            if let Some(active) = &guard.active
                && !Arc::ptr_eq(active, &stale)
            {
                return Ok(active.clone());
            }
            match &guard.dialing {
                Some(token) => {
                    let token = token.clone();
                    drop(guard);
                    wait_settled(&token);
                }
                None => {
                    let token = Arc::new(Dialing {
                        done: Mutex::new(false),
                        cond: Condvar::new(),
                    });
                    guard.dialing = Some(token.clone());
                    drop(guard);

                    let next: Result<Conn, TransportError> =
                        self.dialer.dial(self.budget).map(Arc::from);

                    let mut guard = self.inner.lock().expect("inner mutex");
                    guard.dialing = None;
                    match next {
                        Ok(conn) => {
                            if guard.closed {
                                drop(guard);
                                conn.close();
                                signal(&token);
                                return Err(closed_error());
                            }
                            guard.active = Some(conn.clone());
                            *self.era.lock().expect("era mutex") = Some(conn.era().clone());
                            drop(guard);
                            self.run_rebind(&conn);
                            stale.close();
                            signal(&token);
                            return Ok(conn);
                        }
                        Err(err) => {
                            drop(guard);
                            signal(&token);
                            return Err(err);
                        }
                    }
                }
            }
        }
    }

    fn run_rebind(&self, conn: &Conn) {
        let callbacks = self.rebind.lock().expect("rebind mutex");
        for callback in callbacks.iter() {
            callback(conn.as_ref());
        }
    }
}

fn closed_error() -> TransportError {
    TransportError::Io("mcp connection is closed".into())
}

fn wait_settled(token: &Dialing) {
    let mut done = token.done.lock().expect("dialing mutex");
    while !*done {
        done = token.cond.wait(done).expect("dialing condvar");
    }
}

fn signal(token: &Dialing) {
    let mut done = token.done.lock().expect("dialing mutex");
    *done = true;
    token.cond.notify_all();
}
