use std::collections::HashMap;
use std::fmt;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};

use thiserror::Error;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::sessionstore::eventlog::record::CreatedAt;

use super::barrier::WritePermit;
use super::observer::{BeginWriteError, begin_write_admission, try_write_admission};
use super::store::Store;
use super::types::ActiveWriter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterErrorKind {
    BarrierClosed,
    BarrierBusy,
    Cancelled,
    Panicked,
    Retired,
    Pending,
    Undo,
    InvalidWriterId,
}

#[derive(Debug, Error)]
enum WriterErrorInner {
    #[error("background writer preparation panicked")]
    Panicked,
    #[error("background writer registration retired")]
    Retired,
    #[error("background writer preparation is pending")]
    Pending,
    #[error("writer id must not be empty")]
    InvalidWriterId,
    #[error(transparent)]
    BeginWrite(#[from] BeginWriteError),
}

#[derive(Debug, Clone)]
pub struct WriterError(Arc<WriterErrorInner>);

impl WriterError {
    pub fn kind(&self) -> WriterErrorKind {
        match self.0.as_ref() {
            WriterErrorInner::Panicked => WriterErrorKind::Panicked,
            WriterErrorInner::Retired => WriterErrorKind::Retired,
            WriterErrorInner::Pending => WriterErrorKind::Pending,
            WriterErrorInner::InvalidWriterId => WriterErrorKind::InvalidWriterId,
            WriterErrorInner::BeginWrite(error) => match error {
                BeginWriteError::Cancelled => WriterErrorKind::Cancelled,
                BeginWriteError::Busy => WriterErrorKind::BarrierBusy,
                BeginWriteError::Barrier(super::barrier::BarrierError::Closed) => {
                    WriterErrorKind::BarrierClosed
                }
                BeginWriteError::Barrier(super::barrier::BarrierError::Cancelled) => {
                    WriterErrorKind::Cancelled
                }
                BeginWriteError::Undo(_) => WriterErrorKind::Undo,
            },
        }
    }
}

impl fmt::Display for WriterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for WriterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl From<BeginWriteError> for WriterError {
    fn from(error: BeginWriteError) -> Self {
        Self(Arc::new(WriterErrorInner::BeginWrite(error)))
    }
}

#[derive(Debug, Clone)]
enum RegistrationOutcome {
    Pending,
    Ready,
    Failed(Arc<WriterError>),
}

#[derive(Debug)]
pub(super) struct WriterRegistration {
    writer: ActiveWriter,
    prepare: CancellationToken,
    outcome: watch::Sender<RegistrationOutcome>,
    permit: Mutex<Option<WritePermit>>,
}

impl WriterRegistration {
    fn new(id: &str, kind: &str, turn: i64, prepare: CancellationToken) -> Self {
        let (outcome, _) = watch::channel(RegistrationOutcome::Pending);
        Self {
            writer: ActiveWriter {
                id: id.to_owned(),
                turn,
                started_at: CreatedAt::now(),
                kind: kind.to_owned(),
            },
            prepare,
            outcome,
            permit: Mutex::new(None),
        }
    }

    fn is_active(&self) -> bool {
        matches!(self.outcome.borrow().clone(), RegistrationOutcome::Ready)
            && self
                .permit
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_some()
    }

    fn finish(&self, outcome: RegistrationOutcome) {
        self.outcome.send_replace(outcome);
    }
}

#[derive(Debug, Default)]
pub(super) struct WriterRegistryState {
    writers: HashMap<String, Arc<WriterRegistration>>,
}

#[derive(Debug, Default)]
pub(super) struct WriterRegistry {
    pub(super) state: Mutex<WriterRegistryState>,
    publish: Mutex<()>,
}

impl WriterRegistry {
    fn state(&self) -> MutexGuard<'_, WriterRegistryState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn registration(
        &self,
        writer_id: &str,
        kind: &str,
        turn: i64,
        prepare: CancellationToken,
    ) -> (Arc<WriterRegistration>, bool) {
        let mut state = self.state();
        if let Some(registration) = state.writers.get(writer_id) {
            return (Arc::clone(registration), false);
        }
        let registration = Arc::new(WriterRegistration::new(writer_id, kind, turn, prepare));
        state
            .writers
            .insert(writer_id.to_owned(), Arc::clone(&registration));
        (registration, true)
    }

    #[cfg(test)]
    pub(super) fn contains(&self, writer_id: &str) -> bool {
        self.state().writers.contains_key(writer_id)
    }

    #[cfg(test)]
    pub(super) fn follower_count_for_test(&self, writer_id: &str) -> usize {
        self.state()
            .writers
            .get(writer_id)
            .map_or(0, |registration| registration.outcome.receiver_count())
    }

    pub(super) fn active_writers(&self) -> Vec<ActiveWriter> {
        let state = self.state();
        let mut writers: Vec<_> = state
            .writers
            .values()
            .filter(|registration| registration.is_active())
            .map(|registration| registration.writer.clone())
            .collect();
        writers.sort_by(|left, right| left.id.cmp(&right.id));
        writers
    }

    pub(super) async fn register_async<'s>(
        store: &'s Store,
        registry: Arc<Self>,
        cancellation: &CancellationToken,
        writer_id: &str,
        kind: &str,
        turn: i64,
    ) -> Result<RegisteredWriter<'s>, WriterError> {
        if cancellation.is_cancelled() {
            return Err(WriterError::from(BeginWriteError::Cancelled));
        }
        if writer_id.is_empty() {
            return Err(WriterError(Arc::new(WriterErrorInner::InvalidWriterId)));
        }
        let prepare = cancellation.child_token();
        let (registration, leader) = registry.registration(writer_id, kind, turn, prepare.clone());
        if !leader {
            return wait_for_ready(store, registry, writer_id, cancellation, registration).await;
        }

        let observer = RegistrationPreparation {
            store,
            registry: &registry,
            writer_id,
            registration: &registration,
            prepare: &prepare,
        };
        match catch_unwind_future(observer.begin_write_context(cancellation)).await {
            Ok(Ok(permit)) => observer.publish_admission(permit),
            Ok(Err(error)) => Err(observer.fail(error.into())),
            Err(payload) => {
                observer.fail_panic();
                std::panic::resume_unwind(payload)
            }
        }
    }

    pub(super) fn register_try<'s>(
        store: &'s Store,
        registry: Arc<Self>,
        cancellation: &CancellationToken,
        writer_id: &str,
        kind: &str,
        turn: i64,
    ) -> Result<RegisteredWriter<'s>, WriterError> {
        if cancellation.is_cancelled() {
            return Err(WriterError::from(BeginWriteError::Cancelled));
        }
        if writer_id.is_empty() {
            return Err(WriterError(Arc::new(WriterErrorInner::InvalidWriterId)));
        }
        let prepare = cancellation.child_token();
        let (registration, leader) = registry.registration(writer_id, kind, turn, prepare.clone());
        if !leader {
            let outcome = registration.outcome.borrow().clone();
            return match outcome {
                RegistrationOutcome::Ready
                    if current_registration(&registry, writer_id, &registration) =>
                {
                    Ok(RegisteredWriter::new(
                        store,
                        registry,
                        writer_id,
                        registration,
                    ))
                }
                RegistrationOutcome::Failed(error) => Err(error.as_ref().clone()),
                RegistrationOutcome::Ready => Err(WriterError(Arc::new(WriterErrorInner::Retired))),
                RegistrationOutcome::Pending => {
                    Err(WriterError(Arc::new(WriterErrorInner::Pending)))
                }
            };
        }

        let observer = RegistrationPreparation {
            store,
            registry: &registry,
            writer_id,
            registration: &registration,
            prepare: &prepare,
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            observer.begin_write_try(cancellation)
        })) {
            Ok(Ok(permit)) => observer.publish_admission(permit),
            Ok(Err(error)) => Err(observer.fail(error.into())),
            Err(payload) => {
                observer.fail_panic();
                std::panic::resume_unwind(payload)
            }
        }
    }

    pub(super) fn retire(
        &self,
        store: &Store,
        writer_id: &str,
        expected: Option<&Arc<WriterRegistration>>,
    ) {
        let registration = {
            let mut state = self.state();
            let Some(registration) = state.writers.get(writer_id) else {
                return;
            };
            if expected.is_some_and(|expected| !Arc::ptr_eq(expected, registration)) {
                return;
            }
            state.writers.remove(writer_id)
        };
        let Some(registration) = registration else {
            return;
        };
        registration.prepare.cancel();
        let permit = registration
            .permit
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        drop(permit);
        self.publish(store);
    }

    fn remove_if_current(&self, writer_id: &str, registration: &Arc<WriterRegistration>) {
        let mut state = self.state();
        if state
            .writers
            .get(writer_id)
            .is_some_and(|current| Arc::ptr_eq(current, registration))
        {
            state.writers.remove(writer_id);
        }
    }

    fn publish(&self, store: &Store) {
        let _publication = self
            .publish
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let writers = self.active_writers();
        store.set_active_writers(writers);
    }
}

struct RegistrationPreparation<'store, 'preparation> {
    store: &'store Store,
    registry: &'preparation Arc<WriterRegistry>,
    writer_id: &'preparation str,
    registration: &'preparation Arc<WriterRegistration>,
    prepare: &'preparation CancellationToken,
}

impl<'store, 'preparation> RegistrationPreparation<'store, 'preparation> {
    async fn begin_write_context(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<WritePermit, BeginWriteError> {
        begin_write_admission(self.store, cancellation, self.prepare).await
    }

    fn begin_write_try(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<WritePermit, BeginWriteError> {
        try_write_admission(self.store, cancellation, self.prepare)
    }

    fn publish_admission(
        &self,
        permit: WritePermit,
    ) -> Result<RegisteredWriter<'store>, WriterError> {
        let mut permit = Some(permit);
        let error = {
            let state = self.registry.state();
            if self.prepare.is_cancelled() {
                Some(WriterError::from(BeginWriteError::Cancelled))
            } else if !state
                .writers
                .get(self.writer_id)
                .is_some_and(|current| Arc::ptr_eq(current, self.registration))
            {
                Some(WriterError(Arc::new(WriterErrorInner::Retired)))
            } else {
                *self
                    .registration
                    .permit
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = permit.take();
                self.registration.finish(RegistrationOutcome::Ready);
                None
            }
        };
        if let Some(error) = error {
            drop(permit);
            return Err(self.fail(error));
        }
        self.registry.publish(self.store);
        Ok(RegisteredWriter::new(
            self.store,
            Arc::clone(self.registry),
            self.writer_id,
            Arc::clone(self.registration),
        ))
    }

    fn fail(&self, error: WriterError) -> WriterError {
        self.registry
            .remove_if_current(self.writer_id, self.registration);
        self.prepare.cancel();
        let permit = self
            .registration
            .permit
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        drop(permit);
        self.registry.publish(self.store);
        self.registration
            .finish(RegistrationOutcome::Failed(Arc::new(error.clone())));
        error
    }

    fn fail_panic(&self) {
        self.fail(WriterError(Arc::new(WriterErrorInner::Panicked)));
    }
}

#[derive(Debug)]
pub struct RegisteredWriter<'a> {
    store: &'a Store,
    registry: Arc<WriterRegistry>,
    writer_id: String,
    registration: Arc<WriterRegistration>,
    active: bool,
}

impl<'a> RegisteredWriter<'a> {
    fn new(
        store: &'a Store,
        registry: Arc<WriterRegistry>,
        writer_id: &str,
        registration: Arc<WriterRegistration>,
    ) -> Self {
        Self {
            store,
            registry,
            writer_id: writer_id.to_owned(),
            registration,
            active: true,
        }
    }

    pub fn unregister(&mut self) {
        if self.active {
            self.registry
                .retire(self.store, &self.writer_id, Some(&self.registration));
            self.active = false;
        }
    }
}

impl Drop for RegisteredWriter<'_> {
    fn drop(&mut self) {
        self.unregister();
    }
}

async fn wait_for_ready<'s>(
    store: &'s Store,
    registry: Arc<WriterRegistry>,
    writer_id: &str,
    cancellation: &CancellationToken,
    registration: Arc<WriterRegistration>,
) -> Result<RegisteredWriter<'s>, WriterError> {
    let mut changed = registration.outcome.subscribe();
    loop {
        if cancellation.is_cancelled() {
            return Err(WriterError::from(BeginWriteError::Cancelled));
        }
        match changed.borrow().clone() {
            RegistrationOutcome::Failed(error) => return Err(error.as_ref().clone()),
            RegistrationOutcome::Ready
                if current_registration(&registry, writer_id, &registration) =>
            {
                return Ok(RegisteredWriter::new(
                    store,
                    registry,
                    writer_id,
                    registration,
                ));
            }
            RegistrationOutcome::Ready => {
                return Err(WriterError(Arc::new(WriterErrorInner::Retired)));
            }
            RegistrationOutcome::Pending => {}
        }
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(WriterError::from(BeginWriteError::Cancelled)),
            result = changed.changed() => {
                if result.is_err() {
                    return Err(WriterError(Arc::new(WriterErrorInner::Retired)));
                }
            }
        }
    }
}

fn current_registration(
    registry: &WriterRegistry,
    writer_id: &str,
    registration: &Arc<WriterRegistration>,
) -> bool {
    registry
        .state()
        .writers
        .get(writer_id)
        .is_some_and(|current| Arc::ptr_eq(current, registration))
}

async fn catch_unwind_future<F: Future>(
    future: F,
) -> Result<F::Output, Box<dyn std::any::Any + Send>> {
    let mut future = Box::pin(future);
    poll_fn(|context| {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Pin::new(&mut future).poll(context)
        })) {
            Ok(std::task::Poll::Ready(value)) => std::task::Poll::Ready(Ok(value)),
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Err(payload) => std::task::Poll::Ready(Err(payload)),
        }
    })
    .await
}
