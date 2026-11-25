use std::error::Error;
use std::path::PathBuf;

use thiserror::Error;

use crate::sessionstore::eventlog::record::CreatedAt;
use reasonix_core::fileencoding::UnencodableError;

use super::barrier::BarrierError;
use super::blob::BlobError;
use super::path_capture::{CaptureError, PathError};
use super::rewind::RewindError;
use super::secure_path::SecureError;
use super::store::{Store, StoreError};
use super::types::{FileStage, RewindConflict, RewindResult, RewindScope};

#[cfg(test)]
pub(super) use super::transaction_publish::publish_target;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionPhase {
    Prepare,
    PersistPrepared,
    CrashAfterPrepared,
    PersistCommitting,
    PersistPublishedIntent,
    CrashAfterPublishedIntent,
    PublishFile,
    DeleteFile,
    CrashAfterPublishBeforeProgress,
    PersistProgress,
    BeforeConversation,
    BeforeCheckpointTruncate,
    SyncRestoredOwnership,
    Finalize,
    CrashAfterConversationBeforeFinalize,
    PersistCommitted,
    CompensateFile,
    SyncForwardOwnership,
    PersistAborted,
    Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationPhase {
    ApplyConversationTruncate,
    RestoreConversation,
    TruncateCheckpoints,
    RestoreCheckpoints,
}

#[derive(Debug, Error)]
pub enum ConversationError {
    #[error("injected conversation failure at {0:?}")]
    Injected(ConversationPhase),
    #[error("conversation {phase:?}: {source}")]
    External {
        phase: ConversationPhase,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("conversation {phase:?}: {source}")]
    Operation {
        phase: ConversationPhase,
        #[source]
        source: Box<ConversationError>,
    },
}

impl ConversationError {
    pub fn injected(phase: ConversationPhase) -> Self {
        Self::Injected(phase)
    }

    pub fn external(
        phase: ConversationPhase,
        source: impl Into<Box<dyn Error + Send + Sync>>,
    ) -> Self {
        Self::External {
            phase,
            source: source.into(),
        }
    }

    pub fn phase(&self) -> ConversationPhase {
        match self {
            Self::Injected(phase) => *phase,
            Self::External { phase, .. } | Self::Operation { phase, .. } => *phase,
        }
    }

    pub fn phases(&self) -> Vec<ConversationPhase> {
        let mut phases = Vec::new();
        self.collect_phases(&mut phases);
        phases
    }

    fn collect_phases(&self, phases: &mut Vec<ConversationPhase>) {
        match self {
            Self::Injected(phase) | Self::External { phase, .. } => phases.push(*phase),
            Self::Operation { phase, source } => {
                phases.push(*phase);
                source.collect_phases(phases);
            }
        }
    }
}

pub trait ConversationApplier: Send + Sync {
    fn apply_conversation_truncate(
        &self,
        boundary: i64,
        forward: &[u8],
    ) -> Result<(), ConversationError>;

    fn restore_conversation(&self, forward: &[u8]) -> Result<(), ConversationError>;

    fn truncate_checkpoints(&self, from_turn: i64) -> Result<(), ConversationError>;

    fn restore_checkpoints(&self, backup: &[u8]) -> Result<(), ConversationError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryNote {
    pub phase: RecoveryPhase,
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPhase {
    ReadDirectory,
    ReadManifest,
    AbortPrepared,
    PersistAborted,
    Compensated,
    CompensationPending,
    DeferredConversation,
    UnsupportedKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InjectionKind {
    Fail,
    Crash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Injection {
    phase: TransactionPhase,
    kind: InjectionKind,
    after_files: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransactionInjection {
    entries: Vec<Injection>,
}

impl TransactionInjection {
    pub fn fail(phase: TransactionPhase) -> Self {
        Self::new(InjectionKind::Fail, phase)
    }

    pub fn crash(phase: TransactionPhase) -> Self {
        Self::new(InjectionKind::Crash, phase)
    }

    fn new(kind: InjectionKind, phase: TransactionPhase) -> Self {
        Self {
            entries: vec![Injection {
                phase,
                kind,
                after_files: 0,
            }],
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_after(phase: TransactionPhase, after_files: usize) -> Self {
        Self {
            entries: vec![Injection {
                phase,
                kind: InjectionKind::Fail,
                after_files,
            }],
        }
    }

    pub fn and_fail(mut self, phase: TransactionPhase) -> Self {
        self.entries.push(Injection {
            phase,
            kind: InjectionKind::Fail,
            after_files: 0,
        });
        self
    }

    pub fn and_crash(mut self, phase: TransactionPhase) -> Self {
        self.entries.push(Injection {
            phase,
            kind: InjectionKind::Crash,
            after_files: 0,
        });
        self
    }

    pub(super) fn fails(&self, phase: TransactionPhase) -> bool {
        self.fails_after_files(phase, 0)
    }

    pub(super) fn fails_after_files(&self, phase: TransactionPhase, files_done: usize) -> bool {
        self.entries.iter().any(|entry| {
            entry.phase == phase
                && entry.kind == InjectionKind::Fail
                && files_done >= entry.after_files
        })
    }

    pub(super) fn crashes(&self, phase: TransactionPhase) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.phase == phase && entry.kind == InjectionKind::Crash)
    }
}

#[derive(Debug, Error)]
pub enum TransactionError {
    #[error(transparent)]
    Rewind(#[from] RewindError),
    #[error("rewind plan is disabled")]
    PlanDisabled,
    #[error(transparent)]
    Barrier(#[from] BarrierError),
    #[error("workspace mutation barrier is busy")]
    BarrierBusy,
    #[error("active background writer prevents rewind")]
    ActiveWriters,
    #[error("workspace token changed from {expected} to {actual}")]
    WorkspaceTokenMismatch { expected: String, actual: String },
    #[error("rewind precheck found conflicts")]
    PrecheckConflicts(Vec<RewindConflict>),
    #[error("missing restore payload for {path}")]
    MissingPayload { path: String, reference: String },
    #[error("transaction backup already exists for {path}")]
    BackupAlreadyExists { path: String },
    #[error("transaction backup path is missing for {path}")]
    MissingBackupPath { path: String },
    #[error("transaction publish temp is missing for {path}")]
    MissingPublishTemp { path: String },
    #[error("external modification after publish for {path}")]
    ExternalModification {
        path: String,
        recovery: Option<PathBuf>,
    },
    #[error(transparent)]
    Encoding(#[from] UnencodableError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Secure(#[from] SecureError),
    #[error(transparent)]
    Blob(#[from] BlobError),
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("conversation {phase:?}: {source}")]
    Conversation {
        phase: ConversationPhase,
        #[source]
        source: ConversationError,
    },
    #[error("injected failure at {phase:?}")]
    Injected { phase: TransactionPhase },
    #[error("injected durable-intermediate stop at {phase:?}")]
    InjectedCrash { phase: TransactionPhase },
    #[error("transaction {phase:?}: {source}")]
    Operation {
        phase: TransactionPhase,
        #[source]
        source: Box<TransactionError>,
        stages: Vec<FileStage>,
    },
    #[error("transaction compensation failed: {0:?}")]
    Compensation(Vec<TransactionError>),
    #[error("transaction compensation remains pending: {cause}; {compensation}")]
    CompensationPending {
        cause: Box<TransactionError>,
        compensation: Box<TransactionError>,
        stages: Vec<FileStage>,
    },
}

impl TransactionError {
    pub(super) fn at(phase: TransactionPhase, source: TransactionError) -> Self {
        Self::Operation {
            phase,
            source: Box::new(source),
            stages: Vec::new(),
        }
    }

    pub(super) fn injected(phase: TransactionPhase) -> Self {
        Self::Operation {
            phase,
            source: Box::new(Self::Injected { phase }),
            stages: Vec::new(),
        }
    }

    pub(super) fn with_stages(mut self, stages: Vec<FileStage>) -> Self {
        match &mut self {
            Self::Operation {
                stages: current, ..
            } => *current = stages,
            Self::CompensationPending {
                stages: current, ..
            } => *current = stages,
            _ => {}
        }
        self
    }

    pub fn phases(&self) -> Vec<TransactionPhase> {
        let mut phases = Vec::new();
        self.collect_phases(&mut phases);
        phases
    }

    fn collect_phases(&self, phases: &mut Vec<TransactionPhase>) {
        match self {
            Self::Operation { phase, source, .. } => {
                phases.push(*phase);
                source.collect_phases(phases);
            }
            Self::Compensation(errors) => {
                for error in errors {
                    error.collect_phases(phases);
                }
            }
            Self::CompensationPending {
                cause,
                compensation,
                ..
            } => {
                cause.collect_phases(phases);
                compensation.collect_phases(phases);
            }
            Self::Injected { phase } | Self::InjectedCrash { phase } => phases.push(*phase),
            _ => {}
        }
    }

    pub fn conversation_phases(&self) -> Vec<ConversationPhase> {
        let mut phases = Vec::new();
        self.collect_conversation_phases(&mut phases);
        phases
    }

    fn collect_conversation_phases(&self, phases: &mut Vec<ConversationPhase>) {
        match self {
            Self::Conversation { phase, source } => {
                phases.push(*phase);
                phases.extend(source.phases());
            }
            Self::Operation { source, .. } => source.collect_conversation_phases(phases),
            Self::Compensation(errors) => {
                for error in errors {
                    error.collect_conversation_phases(phases);
                }
            }
            Self::CompensationPending {
                cause,
                compensation,
                ..
            } => {
                cause.collect_conversation_phases(phases);
                compensation.collect_conversation_phases(phases);
            }
            _ => {}
        }
    }
}

impl Store {
    pub fn commit_rewind(
        &self,
        plan_id: &str,
        current_session_revision: i64,
        conversation_forward: &[u8],
        applier: Option<&dyn ConversationApplier>,
        injection: Option<&TransactionInjection>,
    ) -> Result<RewindResult, TransactionError> {
        self.commit_rewind_with_now(
            plan_id,
            current_session_revision,
            conversation_forward,
            applier,
            injection,
            CreatedAt::now(),
        )
    }

    #[cfg(test)]
    pub(crate) fn commit_rewind_at(
        &self,
        plan_id: &str,
        current_session_revision: i64,
        conversation_forward: &[u8],
        applier: Option<&dyn ConversationApplier>,
        injection: Option<&TransactionInjection>,
        now: CreatedAt,
    ) -> Result<RewindResult, TransactionError> {
        self.commit_rewind_with_now(
            plan_id,
            current_session_revision,
            conversation_forward,
            applier,
            injection,
            now,
        )
    }

    fn commit_rewind_with_now(
        &self,
        plan_id: &str,
        current_session_revision: i64,
        conversation_forward: &[u8],
        applier: Option<&dyn ConversationApplier>,
        injection: Option<&TransactionInjection>,
        now: CreatedAt,
    ) -> Result<RewindResult, TransactionError> {
        let plan = self.take_rewind_plan_at(plan_id, now)?;
        if plan.session_revision != current_session_revision {
            return Err(RewindError::SessionRevisionMismatch.into());
        }
        let wants_files = matches!(plan.scope, RewindScope::Code | RewindScope::Both);
        let wants_conversation =
            matches!(plan.scope, RewindScope::Conversation | RewindScope::Both);
        if (wants_files && !plan.can_files) || (wants_conversation && !plan.can_conversation) {
            return Err(TransactionError::PlanDisabled);
        }

        let _permit = self
            .barrier()
            .try_enter_exclusive()
            .ok_or(TransactionError::BarrierBusy)?;
        if !self.state().active_writers.is_empty() {
            return Err(TransactionError::ActiveWriters);
        }
        if wants_files {
            let actual = self.barrier().generation().to_string();
            if plan.workspace_token != actual {
                return Err(TransactionError::WorkspaceTokenMismatch {
                    expected: plan.workspace_token,
                    actual,
                });
            }
            let conflicts = self.precheck_files(plan.turn);
            if !conflicts.is_empty() {
                return Err(TransactionError::PrecheckConflicts(conflicts));
            }
        }

        let mut transaction = super::transaction_prepare::prepare(
            self,
            &plan,
            conversation_forward,
            applier.is_some(),
            injection,
        )?;
        if injection.is_some_and(|value| value.crashes(TransactionPhase::CrashAfterPrepared)) {
            return Err(TransactionError::InjectedCrash {
                phase: TransactionPhase::CrashAfterPrepared,
            });
        }
        super::transaction_publish::commit(self, &mut transaction, applier, injection)
    }
}
