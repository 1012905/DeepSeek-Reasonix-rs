use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use thiserror::Error;
use tokio_util::sync::CancellationToken;

use super::barrier::BarrierError;
use super::store::{Store, StoreError};
use super::types::{
    CaptureAfterOpts, CaptureBeforeOpts, CaptureSource, CoverageGap, PreviewChange,
};
use super::writer_admission::{RegisteredWriter, WriterError, WriterRegistry};

pub struct ObserverOptions<'a> {
    pub store: &'a Store,
    pub ownership_turn: i64,
    pub writer_id: String,
    pub background: bool,
    pub seq: Option<Arc<AtomicI64>>,
}

#[derive(Debug, Error)]
pub enum BeginWriteError {
    #[error("mutation barrier wait cancelled")]
    Cancelled,
    #[error("mutation barrier busy")]
    Busy,
    #[error(transparent)]
    Barrier(#[from] BarrierError),
    #[error(transparent)]
    Undo(#[from] StoreError),
}

pub struct MutationObserver<'a> {
    store: &'a Store,
    registry: Arc<WriterRegistry>,
    ownership_turn: AtomicI64,
    writer_id: String,
    background: bool,
    seq: Arc<AtomicI64>,
}

impl<'a> MutationObserver<'a> {
    pub fn new(options: ObserverOptions<'a>) -> Self {
        Self::with_registry(options, Arc::new(WriterRegistry::default()))
    }

    pub(super) fn with_registry(
        options: ObserverOptions<'a>,
        registry: Arc<WriterRegistry>,
    ) -> Self {
        Self {
            store: options.store,
            registry,
            ownership_turn: AtomicI64::new(options.ownership_turn),
            writer_id: options.writer_id,
            background: options.background,
            seq: options.seq.unwrap_or_default(),
        }
    }

    pub fn store(&self) -> &'a Store {
        self.store
    }

    pub fn writer_id(&self) -> &str {
        &self.writer_id
    }

    pub fn ownership_turn(&self) -> i64 {
        self.ownership_turn.load(Ordering::Relaxed)
    }

    pub fn set_ownership_turn(&self, turn: i64) {
        self.ownership_turn.store(turn, Ordering::Relaxed);
    }

    pub fn background(&self) -> bool {
        self.background
    }

    pub fn clone_for_subagent(
        &self,
        writer_id: impl Into<String>,
        ownership_turn: i64,
        background: bool,
    ) -> Self {
        Self {
            store: self.store,
            registry: Arc::clone(&self.registry),
            ownership_turn: AtomicI64::new(ownership_turn),
            writer_id: writer_id.into(),
            background,
            seq: Arc::clone(&self.seq),
        }
    }

    pub fn before_mutation(&self, path: impl AsRef<Path>, tool: &str, source: CaptureSource) {
        self.store.capture_before(
            path,
            CaptureBeforeOpts {
                tool: Some(tool.to_owned()),
                source: Some(source),
                writer_id: Some(self.writer_id.clone()),
                ownership_turn: Some(self.ownership_turn()),
                background: self.background,
            },
        );
    }

    pub fn before_mutation_from_change(&self, change: PreviewChange, tool: &str) {
        self.store.capture_before_from_change(
            change,
            CaptureBeforeOpts {
                tool: Some(tool.to_owned()),
                source: Some(CaptureSource::Previewer),
                writer_id: Some(self.writer_id.clone()),
                ownership_turn: Some(self.ownership_turn()),
                background: self.background,
            },
        );
    }

    pub fn after_mutation(&self, path: impl AsRef<Path>, tool: &str) -> Result<(), StoreError> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        self.store.capture_after(
            path,
            CaptureAfterOpts {
                seq,
                tool: Some(tool.to_owned()),
                source: Some(CaptureSource::AfterMutation),
                writer_id: Some(self.writer_id.clone()),
                ownership_turn: Some(self.ownership_turn()),
                background: self.background,
            },
        )
    }

    pub fn record_gap(&self, gap: CoverageGap) {
        self.store.record_gap(gap);
    }

    pub fn note_cross_turn_background_writer(&self, new_turn: i64) {
        for writer in self.active_writers() {
            if writer.turn < new_turn {
                self.record_gap(CoverageGap {
                    path: String::new(),
                    reason: super::types::GAP_BACKGROUND_WRITER.to_owned(),
                    detail: "background writer from earlier turn still active".to_owned(),
                    tool: writer.kind,
                });
            }
        }
    }

    pub fn active_writers(&self) -> Vec<super::types::ActiveWriter> {
        self.registry.active_writers()
    }

    pub fn has_other_active_writers(&self) -> bool {
        self.active_writers()
            .iter()
            .any(|writer| writer.id != self.writer_id)
    }

    pub fn has_active_writers(&self) -> bool {
        !self.active_writers().is_empty()
    }

    pub async fn begin_write(&self) -> Result<super::barrier::WritePermit, BeginWriteError> {
        self.begin_write_context(&CancellationToken::new()).await
    }

    pub async fn begin_write_context(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<super::barrier::WritePermit, BeginWriteError> {
        begin_write_admission(self.store, cancellation, cancellation).await
    }

    pub fn try_begin_write(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<super::barrier::WritePermit, BeginWriteError> {
        try_write_admission(self.store, cancellation, cancellation)
    }

    pub async fn register_writer_context(
        &self,
        cancellation: &CancellationToken,
        writer_id: &str,
        kind: &str,
        turn: i64,
    ) -> Result<RegisteredWriter<'a>, WriterError> {
        WriterRegistry::register_async(
            self.store,
            Arc::clone(&self.registry),
            cancellation,
            writer_id,
            kind,
            turn,
        )
        .await
    }

    pub fn try_register_writer(
        &self,
        writer_id: &str,
        kind: &str,
        turn: i64,
    ) -> Result<RegisteredWriter<'a>, WriterError> {
        WriterRegistry::register_try(
            self.store,
            Arc::clone(&self.registry),
            &CancellationToken::new(),
            writer_id,
            kind,
            turn,
        )
    }

    pub fn unregister_writer(&self, writer_id: &str) {
        self.registry.retire(self.store, writer_id, None);
    }

    #[cfg(test)]
    pub(super) fn has_writer_registration_for_test(&self, writer_id: &str) -> bool {
        self.registry.contains(writer_id)
    }

    #[cfg(test)]
    pub(super) fn writer_followers_for_test(&self, writer_id: &str) -> usize {
        self.registry.follower_count_for_test(writer_id)
    }
}

pub(super) async fn begin_write_admission(
    store: &Store,
    caller: &CancellationToken,
    preparation: &CancellationToken,
) -> Result<super::barrier::WritePermit, BeginWriteError> {
    if caller.is_cancelled() || preparation.is_cancelled() {
        return Err(BeginWriteError::Cancelled);
    }
    let permit = tokio::select! {
        biased;
        _ = preparation.cancelled() => return Err(BeginWriteError::Cancelled),
        result = store.barrier().enter_write() => result?,
    };
    store.invalidate_undo()?;
    if caller.is_cancelled() || preparation.is_cancelled() {
        return Err(BeginWriteError::Cancelled);
    }
    Ok(permit)
}

pub(super) fn try_write_admission(
    store: &Store,
    caller: &CancellationToken,
    preparation: &CancellationToken,
) -> Result<super::barrier::WritePermit, BeginWriteError> {
    if caller.is_cancelled() || preparation.is_cancelled() {
        return Err(BeginWriteError::Cancelled);
    }
    let permit = store.barrier().try_write()?.ok_or(BeginWriteError::Busy)?;
    store.invalidate_undo()?;
    if caller.is_cancelled() || preparation.is_cancelled() {
        return Err(BeginWriteError::Cancelled);
    }
    Ok(permit)
}
