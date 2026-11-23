//! Bounded permit pools shared by the writer, notification pool, and handler
//! slots.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Remaining time before `deadline`, `None` once it has passed.
pub(crate) fn remaining(deadline: Instant) -> Option<Duration> {
    let now = Instant::now();
    (now < deadline).then(|| deadline - now)
}

/// Bounded permit pool; waiters wake on release and on close.
pub(crate) struct Gate {
    state: Mutex<GateState>,
    cv: Condvar,
}

#[derive(Default)]
struct GateState {
    free: usize,
    closed: bool,
}

pub(crate) enum GateError {
    TimedOut,
    Closed,
}

impl Gate {
    pub(crate) fn new(permits: usize) -> Self {
        Self {
            state: Mutex::new(GateState {
                free: permits,
                closed: false,
            }),
            cv: Condvar::new(),
        }
    }

    pub(crate) fn try_acquire(&self) -> bool {
        let mut state = self.state.lock().expect("gate mutex");
        if state.closed || state.free == 0 {
            return false;
        }
        state.free -= 1;
        true
    }

    pub(crate) fn acquire(&self, deadline: Option<Instant>) -> Result<(), GateError> {
        let mut state = self.state.lock().expect("gate mutex");
        loop {
            if state.closed {
                return Err(GateError::Closed);
            }
            if state.free > 0 {
                state.free -= 1;
                return Ok(());
            }
            match deadline {
                None => state = self.cv.wait(state).expect("gate mutex"),
                Some(deadline) => match remaining(deadline) {
                    Some(left) => {
                        let (next, _) = self.cv.wait_timeout(state, left).expect("gate mutex");
                        state = next;
                    }
                    None => return Err(GateError::TimedOut),
                },
            }
        }
    }

    pub(crate) fn release(&self) {
        let mut state = self.state.lock().expect("gate mutex");
        state.free += 1;
        self.cv.notify_one();
    }

    pub(crate) fn close(&self) {
        let mut state = self.state.lock().expect("gate mutex");
        state.closed = true;
        self.cv.notify_all();
    }
}
