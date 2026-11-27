use std::collections::BTreeMap;
use std::path::Path;

use thiserror::Error;

use crate::sessionstore::eventlog::record::CreatedAt;

use super::path_capture::{
    CaptureError, Fingerprint, capture_path, compare_identity, normalize_rel_path, safe_path,
    same_fingerprint,
};
use super::rewind::{is_expired, new_plan_id};
use super::store::Store;
use super::transaction::{TransactionError, TransactionInjection, TransactionPhase};
use super::types::{
    CONFLICT_BOUNDARY_INVALID, CONFLICT_BUSY_WRITER, CONFLICT_COVERAGE_LEGACY, CONFLICT_EXPIRED,
    CONFLICT_EXTERNAL_CHANGE, CONFLICT_MISSING_PAYLOAD, CONFLICT_PATH_UNSAFE, CONFLICT_STALE_PLAN,
    CaptureOptions, ConflictResolution, Coverage, CoverageGap, DISABLED_EXPIRED, DISABLED_LEGACY,
    EXPIRED_DETAIL, FileRevision, GAP_EXPIRED_PAYLOAD, RewindConflict, RewindPlan, RewindResult,
    RewindScope, SCHEMA_V2,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FileRevertDisabledReason {
    #[error("file is not session-owned")]
    NotSessionOwned,
    #[error("path unsafe")]
    PathUnsafe,
    #[error("legacy checkpoint cannot verify later manual edits")]
    LegacyOwnership,
    #[error("legacy checkpoint cannot verify later manual edits")]
    IncompleteOwnership,
    #[error("missing file payload")]
    MissingPayload,
    #[error("file recovery payload expired")]
    ExpiredPayload,
    #[error("current file identity unavailable")]
    CurrentIdentityUnavailable,
}

#[derive(Debug, Error)]
pub enum FileRevertRejection {
    #[error("plan disabled: {0}")]
    Disabled(FileRevertDisabledReason),
    #[error("undo recovery is pending")]
    UndoRecoveryPending,
    #[error("workspace mutation in progress")]
    BarrierBusy,
    #[error("active background writer")]
    ActiveWriters,
    #[error("workspace changed since preview")]
    WorkspaceChanged { expected: String, actual: String },
    #[error("file is no longer session-owned")]
    OwnershipLost,
    #[error(transparent)]
    PathUnsafe(#[from] super::path_capture::PathError),
    #[error("file changed since preview; preview again")]
    PreviewChanged {
        expected: Box<Fingerprint>,
        current: Box<Fingerprint>,
    },
    #[error(transparent)]
    PreviewUnavailable(#[from] CaptureError),
}

#[derive(Debug, Error)]
pub enum FileRevertError {
    #[error("unknown or expired plan")]
    UnknownOrExpiredPlan,
    #[error("file revert rejected: {reason}")]
    Rejected {
        #[source]
        reason: FileRevertRejection,
        result: Box<RewindResult>,
    },
    #[error(transparent)]
    Transaction(#[from] TransactionError),
}

#[derive(Debug)]
pub(super) struct PreparedFileRewind {
    pub(super) plan: RewindPlan,
    pub(super) preview: Option<Fingerprint>,
    pub(super) disabled: Option<FileRevertDisabledReason>,
    pub(super) prepared_at: CreatedAt,
}

#[derive(Debug, Default)]
pub(super) struct FilePlanRegistry {
    pub(super) entries: BTreeMap<String, PreparedFileRewind>,
}

impl FilePlanRegistry {
    pub(super) fn take(&mut self, plan_id: &str) -> Option<PreparedFileRewind> {
        self.entries.remove(plan_id)
    }

    pub(super) fn drop_stale(&mut self, now: CreatedAt) {
        self.entries
            .retain(|_, prepared| !is_expired(prepared.prepared_at, now));
    }
}

pub(super) struct FileRevisionSelection {
    pub(super) revision: FileRevision,
    pub(super) legacy: bool,
    pub(super) expired: bool,
    pub(super) coverage: Coverage,
    pub(super) gaps: Vec<CoverageGap>,
}

impl Store {
    pub fn prepare_file_revert(&self, path: impl AsRef<Path>, session_revision: i64) -> RewindPlan {
        let path = path.as_ref();
        let created_at = CreatedAt::now();
        let mut plan = RewindPlan {
            plan_id: new_plan_id(created_at),
            scope: RewindScope::Code,
            session_revision,
            workspace_token: self.barrier().generation().to_string(),
            created_at,
            path: path.to_string_lossy().into_owned(),
            files: vec![path.to_string_lossy().into_owned()],
            file_count: 1,
            ..RewindPlan::default()
        };
        if self.file_state(path).is_none() {
            plan.can_files = false;
            plan.disabled_reason = FileRevertDisabledReason::NotSessionOwned.to_string();
            return plan;
        }
        let absolute = match safe_path(&self.root, path) {
            Ok(absolute) => absolute,
            Err(_) => {
                plan.can_files = false;
                plan.disabled_reason = FileRevertDisabledReason::PathUnsafe.to_string();
                plan.conflicts
                    .push(conflict(&plan.path, CONFLICT_PATH_UNSAFE));
                return plan;
            }
        };
        let Some(selection) = self.earliest_file_revision(path) else {
            plan.can_files = false;
            plan.disabled_reason = FileRevertDisabledReason::NotSessionOwned.to_string();
            return plan;
        };
        plan.path = selection.revision.path.clone();
        plan.coverage = selection.coverage;
        plan.coverage_gaps = selection.gaps.clone();
        plan.legacy = selection.legacy || selection.coverage == Coverage::Legacy;
        plan.expired_file_payload = selection.expired;
        if plan.legacy || !ownership_complete(&selection.revision) {
            refuse_unverified(&mut plan);
            return plan;
        }
        if selection.expired {
            plan.coverage = Coverage::Partial;
            add_gap(
                &mut plan.coverage_gaps,
                GAP_EXPIRED_PAYLOAD,
                EXPIRED_DETAIL,
                &plan.path,
            );
        } else if !selection.gaps.is_empty() && plan.coverage == Coverage::Complete {
            plan.coverage = Coverage::Partial;
        }

        let mut preview = None;
        let mut disabled = None;
        match capture_current(self, &absolute) {
            Ok(current) => {
                if let Some(reason) = compare_identity(
                    &current,
                    &selection.revision.after_sha256,
                    selection.revision.after_existed,
                    selection.revision.after_mode,
                ) {
                    plan.can_files = true;
                    plan.disabled_reason = "conflict requires explicit resolution".to_owned();
                    plan.conflicts.push(identity_conflict(
                        &plan.files[0],
                        &selection.revision,
                        &current,
                        reason,
                    ));
                } else {
                    plan.can_files = true;
                }
                preview = Some(current);
            }
            Err(error) => {
                plan.can_files = false;
                plan.coverage = Coverage::Partial;
                plan.disabled_reason =
                    FileRevertDisabledReason::CurrentIdentityUnavailable.to_string();
                let reason = if error.is_outside_workspace() {
                    CONFLICT_PATH_UNSAFE
                } else {
                    CONFLICT_EXTERNAL_CHANGE
                };
                plan.conflicts.push(conflict(&plan.files[0], reason));
                let mut gap = error.gap();
                gap.path = plan.path.clone();
                plan.coverage_gaps.push(gap);
                disabled = Some(FileRevertDisabledReason::CurrentIdentityUnavailable);
            }
        }

        if selection.expired {
            plan.can_files = false;
            plan.disabled_reason = DISABLED_EXPIRED.to_owned();
            plan.conflicts.push(conflict(&plan.path, CONFLICT_EXPIRED));
            disabled = Some(FileRevertDisabledReason::ExpiredPayload);
        } else if !payload_available(self, &selection.revision) {
            plan.can_files = false;
            plan.disabled_reason = FileRevertDisabledReason::MissingPayload.to_string();
            plan.conflicts
                .push(conflict(&plan.files[0], CONFLICT_MISSING_PAYLOAD));
            disabled = Some(FileRevertDisabledReason::MissingPayload);
        }
        self.store_file_plan(plan.clone(), preview, disabled);
        plan
    }

    pub fn commit_file_revert(
        &self,
        plan_id: &str,
        resolution: ConflictResolution,
    ) -> Result<RewindResult, FileRevertError> {
        self.commit_file_revert_inner(plan_id, resolution, None)
    }

    #[cfg(test)]
    pub(crate) fn commit_file_revert_with_injection(
        &self,
        plan_id: &str,
        resolution: ConflictResolution,
        injection: Option<&TransactionInjection>,
    ) -> Result<RewindResult, FileRevertError> {
        self.commit_file_revert_inner(plan_id, resolution, injection)
    }

    fn commit_file_revert_inner(
        &self,
        plan_id: &str,
        resolution: ConflictResolution,
        injection: Option<&TransactionInjection>,
    ) -> Result<RewindResult, FileRevertError> {
        let prepared = {
            let mut registry = self.plans();
            registry.files.take(plan_id)
        }
        .ok_or(FileRevertError::UnknownOrExpiredPlan)?;
        let plan = prepared.plan;
        if let Some(disabled) = prepared.disabled {
            return Err(reject(
                &plan,
                FileRevertRejection::Disabled(disabled),
                plan.conflicts.clone(),
                Vec::new(),
            ));
        }
        if plan.path.is_empty() || !plan.can_files {
            return Err(reject(
                &plan,
                FileRevertRejection::Disabled(FileRevertDisabledReason::NotSessionOwned),
                plan.conflicts.clone(),
                Vec::new(),
            ));
        }
        if resolution == ConflictResolution::KeepCurrent && !plan.conflicts.is_empty() {
            return Ok(RewindResult {
                ok: true,
                ..RewindResult::default()
            });
        }

        let _permit = self.barrier().try_enter_exclusive().ok_or_else(|| {
            reject(
                &plan,
                FileRevertRejection::BarrierBusy,
                vec![conflict(&plan.path, CONFLICT_BUSY_WRITER)],
                Vec::new(),
            )
        })?;
        if self.require_undo_recovery_complete().is_err() {
            return Err(reject(
                &plan,
                FileRevertRejection::UndoRecoveryPending,
                vec![conflict(&plan.path, CONFLICT_BOUNDARY_INVALID)],
                Vec::new(),
            ));
        }
        let writer_conflicts = self.active_writer_conflicts();
        if !writer_conflicts.is_empty() {
            return Err(reject(
                &plan,
                FileRevertRejection::ActiveWriters,
                writer_conflicts,
                Vec::new(),
            ));
        }
        let actual = self.barrier().generation().to_string();
        if plan.workspace_token != actual {
            return Err(reject(
                &plan,
                FileRevertRejection::WorkspaceChanged {
                    expected: plan.workspace_token.clone(),
                    actual,
                },
                vec![conflict(&plan.path, CONFLICT_STALE_PLAN)],
                Vec::new(),
            ));
        }
        let Some(preview) = prepared.preview else {
            return Err(reject(
                &plan,
                FileRevertRejection::Disabled(FileRevertDisabledReason::CurrentIdentityUnavailable),
                plan.conflicts.clone(),
                Vec::new(),
            ));
        };
        let absolute = safe_path(&self.root, Path::new(&plan.path)).map_err(|error| {
            reject(
                &plan,
                FileRevertRejection::PathUnsafe(error),
                vec![conflict(&plan.path, CONFLICT_PATH_UNSAFE)],
                Vec::new(),
            )
        })?;
        let current = capture_current(self, &absolute).map_err(|error| {
            let mut gaps = vec![error.gap()];
            gaps[0].path = plan.path.clone();
            reject(
                &plan,
                FileRevertRejection::PreviewUnavailable(error),
                vec![conflict(&plan.path, CONFLICT_STALE_PLAN)],
                gaps,
            )
        })?;
        if !same_fingerprint(&preview, &current) {
            return Err(reject(
                &plan,
                FileRevertRejection::PreviewChanged {
                    expected: Box::new(preview),
                    current: Box::new(current.clone()),
                },
                vec![identity_conflict(
                    &plan.path,
                    &FileRevision::default(),
                    &current,
                    CONFLICT_STALE_PLAN,
                )],
                Vec::new(),
            ));
        }
        let selection = self.earliest_file_revision(&absolute).ok_or_else(|| {
            reject(
                &plan,
                FileRevertRejection::OwnershipLost,
                vec![conflict(&plan.path, CONFLICT_EXTERNAL_CHANGE)],
                Vec::new(),
            )
        })?;
        if selection.legacy
            || selection.coverage == Coverage::Legacy
            || !ownership_complete(&selection.revision)
        {
            let disabled = if selection.legacy || selection.coverage == Coverage::Legacy {
                FileRevertDisabledReason::LegacyOwnership
            } else {
                FileRevertDisabledReason::IncompleteOwnership
            };
            return Err(reject(
                &plan,
                FileRevertRejection::Disabled(disabled),
                vec![conflict(&plan.path, CONFLICT_COVERAGE_LEGACY)],
                Vec::new(),
            ));
        }
        if selection.expired {
            return Err(reject(
                &plan,
                FileRevertRejection::Disabled(FileRevertDisabledReason::ExpiredPayload),
                vec![conflict(&plan.path, CONFLICT_EXPIRED)],
                Vec::new(),
            ));
        }
        if !payload_available(self, &selection.revision) {
            return Err(reject(
                &plan,
                FileRevertRejection::Disabled(FileRevertDisabledReason::MissingPayload),
                vec![conflict(&plan.path, CONFLICT_MISSING_PAYLOAD)],
                Vec::new(),
            ));
        }
        let mut transaction = super::transaction_prepare::prepare_file_revert(
            self,
            &plan,
            &selection.revision,
            injection,
        )?;
        if injection.is_some_and(|value| value.crashes(TransactionPhase::CrashAfterPrepared)) {
            return Err(TransactionError::InjectedCrash {
                phase: TransactionPhase::CrashAfterPrepared,
            }
            .into());
        }
        super::transaction_publish::commit(self, &mut transaction, None, injection)
            .map_err(Into::into)
    }

    fn store_file_plan(
        &self,
        plan: RewindPlan,
        preview: Option<Fingerprint>,
        disabled: Option<FileRevertDisabledReason>,
    ) {
        let prepared_at = CreatedAt::now();
        let mut registry = self.plans();
        registry.files.entries.insert(
            plan.plan_id.clone(),
            PreparedFileRewind {
                plan,
                preview,
                disabled,
                prepared_at,
            },
        );
        registry.drop_stale(prepared_at);
    }

    pub(super) fn earliest_file_revision(&self, path: &Path) -> Option<FileRevisionSelection> {
        let absolute = safe_path(&self.root, path).ok()?;
        let checkpoints = self.state().all();
        let mut matches = Vec::new();
        for checkpoint in &checkpoints {
            for revision in checkpoint.revisions() {
                let Ok(recorded) = safe_path(&self.root, Path::new(&revision.path)) else {
                    continue;
                };
                if recorded == absolute {
                    matches.push((checkpoint, revision));
                }
            }
        }
        let (first_checkpoint, first_revision) = matches.first()?;
        let (_, last_revision) = matches.last()?;
        let mut revision = first_revision.clone();
        revision.path = normalize_rel_path(&self.root, Path::new(&revision.path));
        revision.after_existed = last_revision.after_existed;
        revision.after_sha256 = last_revision.after_sha256.clone();
        revision.after_mode = last_revision.after_mode;
        let legacy = first_checkpoint.legacy || first_checkpoint.schema_version < SCHEMA_V2;
        let coverage = if first_checkpoint.coverage == Coverage::Unspecified {
            Coverage::Complete
        } else {
            first_checkpoint.coverage
        };
        Some(FileRevisionSelection {
            revision,
            legacy,
            expired: first_checkpoint.expired_file_payload,
            coverage,
            gaps: first_checkpoint.coverage_gaps.clone(),
        })
    }
}

fn capture_current(store: &Store, absolute: &Path) -> Result<Fingerprint, CaptureError> {
    capture_path(
        absolute,
        &CaptureOptions {
            workspace_root: store.root.clone(),
            read_content: true,
            max_bytes: store.config.max_file_bytes,
        },
    )
}

fn ownership_complete(revision: &FileRevision) -> bool {
    match revision.after_existed {
        Some(true) => !revision.after_sha256.is_empty(),
        Some(false) => revision.after_sha256.is_empty(),
        None => false,
    }
}

fn payload_available(store: &Store, revision: &FileRevision) -> bool {
    if !revision.existed {
        return true;
    }
    if !revision.blob_ref.is_empty() {
        return store
            .blobs
            .as_ref()
            .is_some_and(|blobs| blobs.has(&revision.blob_ref))
            || revision.content.is_some();
    }
    if revision.content.is_some() {
        return true;
    }
    !revision.sha256.is_empty()
        && store
            .blobs
            .as_ref()
            .is_some_and(|blobs| blobs.has(&revision.sha256))
}

fn refuse_unverified(plan: &mut RewindPlan) {
    plan.plan_id.clear();
    plan.can_files = false;
    plan.legacy = true;
    plan.coverage = Coverage::Legacy;
    plan.disabled_reason = DISABLED_LEGACY.to_owned();
}

fn add_gap(gaps: &mut Vec<CoverageGap>, reason: &str, detail: &str, path: &str) {
    if gaps.iter().any(|gap| gap.reason == reason) {
        return;
    }
    let mut gap = CoverageGap::new(reason, detail);
    gap.path = path.to_owned();
    gaps.push(gap);
}

fn conflict(path: &str, reason: &str) -> RewindConflict {
    RewindConflict {
        path: path.to_owned(),
        reason: reason.to_owned(),
        ..RewindConflict::default()
    }
}

fn identity_conflict(
    path: &str,
    revision: &FileRevision,
    current: &Fingerprint,
    reason: &str,
) -> RewindConflict {
    RewindConflict {
        path: path.to_owned(),
        reason: reason.to_owned(),
        checkpoint_sha: revision.sha256.clone(),
        last_owned_sha: revision.after_sha256.clone(),
        current_sha: current.sha256.clone(),
        checkpoint_mode: revision.mode,
        current_mode: current.mode,
        current_existed: current.existed,
        checkpoint_existed: revision.existed,
    }
}

fn reject(
    plan: &RewindPlan,
    reason: FileRevertRejection,
    conflicts: Vec<RewindConflict>,
    extra_gaps: Vec<CoverageGap>,
) -> FileRevertError {
    let mut coverage = plan.coverage;
    let mut gaps = plan.coverage_gaps.clone();
    if !extra_gaps.is_empty() && coverage == Coverage::Complete {
        coverage = Coverage::Partial;
    }
    gaps.extend(extra_gaps);
    let message = reason.to_string();
    FileRevertError::Rejected {
        reason,
        result: Box::new(RewindResult {
            error: message,
            conflicts,
            coverage,
            coverage_gaps: gaps,
            ..RewindResult::default()
        }),
    }
}
