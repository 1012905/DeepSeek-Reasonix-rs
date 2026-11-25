use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use thiserror::Error;
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BarrierError {
    #[error("mutation barrier closed")]
    Closed,
    #[error("mutation barrier wait cancelled")]
    Cancelled,
}

#[derive(Debug, Default)]
struct BarrierState {
    writers: usize,
    exclusive: bool,
    closed: bool,
}

#[derive(Debug)]
struct BarrierInner {
    state: Mutex<BarrierState>,
    changed: Notify,
    generation: AtomicU64,
}

impl BarrierInner {
    fn state(&self) -> MutexGuard<'_, BarrierState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn exit_write(&self) {
        let wake = {
            let mut state = self.state();
            if state.writers == 0 {
                return;
            }
            state.writers -= 1;
            self.generation.fetch_add(1, Ordering::Relaxed);
            state.writers == 0
        };
        if wake {
            self.changed.notify_waiters();
        }
    }

    fn exit_exclusive(&self) {
        {
            let mut state = self.state();
            state.exclusive = false;
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
        self.changed.notify_waiters();
    }
}

#[derive(Clone, Debug)]
pub struct MutationBarrier {
    inner: Arc<BarrierInner>,
}

impl MutationBarrier {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(BarrierInner {
                state: Mutex::new(BarrierState::default()),
                changed: Notify::new(),
                generation: AtomicU64::new(0),
            }),
        }
    }

    pub fn generation(&self) -> u64 {
        self.inner.generation.load(Ordering::Relaxed)
    }

    pub fn has_writers(&self) -> bool {
        self.inner.state().writers > 0
    }

    pub fn busy(&self) -> bool {
        let state = self.inner.state();
        state.exclusive || state.writers > 0
    }

    pub fn close(&self) {
        self.inner.state().closed = true;
        self.inner.changed.notify_waiters();
    }

    pub fn try_write(&self) -> Result<Option<WritePermit>, BarrierError> {
        self.try_write_inner()
    }

    pub fn try_enter_write(&self) -> Option<WritePermit> {
        self.try_write().ok().flatten()
    }

    pub fn try_enter_exclusive(&self) -> Option<ExclusivePermit> {
        self.try_exclusive().ok().flatten()
    }

    pub async fn enter_write(&self) -> Result<WritePermit, BarrierError> {
        self.wait_for_admission(|| self.try_write()).await
    }

    pub async fn enter_exclusive(&self) -> Result<ExclusivePermit, BarrierError> {
        self.wait_for_admission(|| self.try_exclusive()).await
    }

    fn try_write_inner(&self) -> Result<Option<WritePermit>, BarrierError> {
        let mut state = self.inner.state();
        if state.closed {
            return Err(BarrierError::Closed);
        }
        if state.exclusive {
            return Ok(None);
        }
        state.writers += 1;
        drop(state);
        Ok(Some(WritePermit {
            inner: Some(self.inner.clone()),
        }))
    }

    fn try_exclusive(&self) -> Result<Option<ExclusivePermit>, BarrierError> {
        let mut state = self.inner.state();
        if state.closed {
            return Err(BarrierError::Closed);
        }
        if state.exclusive || state.writers > 0 {
            return Ok(None);
        }
        state.exclusive = true;
        drop(state);
        Ok(Some(ExclusivePermit {
            inner: Some(self.inner.clone()),
        }))
    }

    pub(super) async fn wait_for_admission<T>(
        &self,
        mut attempt: impl FnMut() -> Result<Option<T>, BarrierError>,
    ) -> Result<T, BarrierError> {
        loop {
            let notified = self.inner.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match attempt()? {
                Some(permit) => return Ok(permit),
                None => notified.await,
            }
        }
    }
}

impl Default for MutationBarrier {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct WritePermit {
    inner: Option<Arc<BarrierInner>>,
}

impl WritePermit {
    pub fn release(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.exit_write();
        }
    }
}

impl Drop for WritePermit {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Debug)]
pub struct ExclusivePermit {
    inner: Option<Arc<BarrierInner>>,
}

impl ExclusivePermit {
    pub fn release(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.exit_exclusive();
        }
    }
}

impl Drop for ExclusivePermit {
    fn drop(&mut self) {
        self.release();
    }
}
