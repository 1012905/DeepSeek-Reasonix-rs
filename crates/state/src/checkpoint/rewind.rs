use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::MutexGuard;

use chrono::Duration;
use thiserror::Error;

use crate::sessionstore::eventlog::record::CreatedAt;

use super::blob::digest;
use super::path_capture::{compare_identity, fingerprint_path, normalize_rel_path, safe_path};
use super::store::Store;
use super::types::{
    CONFLICT_BUSY_WRITER, CONFLICT_COVERAGE_LEGACY, CONFLICT_EXPIRED, CONFLICT_EXTERNAL_CHANGE,
    CONFLICT_MISSING_PAYLOAD, CONFLICT_PATH_UNSAFE, Checkpoint, Coverage, CoverageGap,
    DISABLED_EXPIRED, DISABLED_LEGACY, EXPIRED_DETAIL, FileRevision, GAP_EXPIRED_PAYLOAD,
    GAP_LEGACY_UNVERIFIED, RewindConflict, RewindPlan, RewindScope, SCHEMA_V2,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RewindError {
    #[error("unknown or expired plan")]
    UnknownOrExpiredPlan,
    #[error("conversation changed since preview")]
    SessionRevisionMismatch,
}

#[derive(Debug, Clone)]
pub(super) struct PreparedRewind {
    pub(super) plan: RewindPlan,
    pub(super) prepared_at: CreatedAt,
}

#[derive(Debug, Default)]
pub(super) struct PlanRegistry {
    pub(super) entries: BTreeMap<String, PreparedRewind>,
}

impl PlanRegistry {
    fn get(&mut self, plan_id: &str, now: CreatedAt) -> Result<RewindPlan, RewindError> {
        let Some(prepared) = self.entries.get(plan_id) else {
            return Err(RewindError::UnknownOrExpiredPlan);
        };
        if is_expired(prepared.prepared_at, now) {
            self.entries.remove(plan_id);
            return Err(RewindError::UnknownOrExpiredPlan);
        }
        Ok(prepared.plan.clone())
    }

    fn drop_stale(&mut self, now: CreatedAt) {
        self.entries
            .retain(|_, prepared| !is_expired(prepared.prepared_at, now));
    }
}

fn is_expired(prepared_at: CreatedAt, now: CreatedAt) -> bool {
    now.get().signed_duration_since(prepared_at.get()) > Duration::minutes(10)
}

impl Store {
    pub fn prepare_rewind(
        &self,
        turn: i64,
        scope: RewindScope,
        session_revision: i64,
        boundary: i64,
        has_boundary: bool,
    ) -> RewindPlan {
        let created_at = CreatedAt::now();
        let mut plan = RewindPlan {
            plan_id: new_plan_id(created_at),
            turn,
            scope,
            session_revision,
            workspace_token: self.barrier().generation().to_string(),
            boundary_index: boundary,
            has_boundary,
            created_at,
            ..RewindPlan::default()
        };
        let (writers, files, coverage, gaps, legacy, expired) = {
            let state = self.state();
            let checkpoints = state.all();
            let writers = state.active_writers.clone();
            let (coverage, gaps, legacy, expired) = coverage_from_turn(&checkpoints, turn);
            let files = files_from_turn(&self.root, &checkpoints, turn);
            (writers, files, coverage, gaps, legacy, expired)
        };
        plan.active_writers = writers.clone();
        plan.files = files.clone();
        plan.file_count = files.len();
        plan.coverage = coverage;
        plan.coverage_gaps = gaps;
        plan.legacy = legacy;
        plan.expired_file_payload = expired;

        if !writers.is_empty() {
            plan.can_files = false;
            plan.can_conversation = false;
            plan.disabled_reason = "active background writer".to_owned();
            plan.conflicts = writers
                .iter()
                .map(|_| RewindConflict {
                    reason: CONFLICT_BUSY_WRITER.to_owned(),
                    ..RewindConflict::default()
                })
                .collect();
            return plan;
        }

        let want_files = matches!(scope, RewindScope::Code | RewindScope::Both);
        let want_conversation = matches!(scope, RewindScope::Conversation | RewindScope::Both);
        if want_conversation {
            plan.can_conversation = has_boundary;
            if !has_boundary {
                plan.disabled_reason = "conversation boundary unavailable".to_owned();
            }
        }

        if want_files {
            if files.is_empty() && scope == RewindScope::Both {
                plan.can_files = true;
            } else if coverage == Coverage::None {
                plan.disabled_reason = "no file captures".to_owned();
            } else if expired {
                plan.can_files = false;
                plan.disabled_reason = DISABLED_EXPIRED.to_owned();
                plan.conflicts.push(RewindConflict {
                    reason: CONFLICT_EXPIRED.to_owned(),
                    ..RewindConflict::default()
                });
            } else if legacy {
                plan.can_files = false;
                plan.disabled_reason = DISABLED_LEGACY.to_owned();
                plan.conflicts.push(RewindConflict {
                    reason: CONFLICT_COVERAGE_LEGACY.to_owned(),
                    ..RewindConflict::default()
                });
            } else {
                let conflicts = self.precheck_files(turn);
                plan.can_files = conflicts.is_empty() && !files.is_empty();
                if !conflicts.is_empty() {
                    plan.disabled_reason = "file conflicts detected".to_owned();
                    plan.conflicts = conflicts;
                }
            }
        }

        if scope == RewindScope::Both
            && (!plan.can_files || !plan.can_conversation)
            && plan.disabled_reason.is_empty()
        {
            plan.disabled_reason = "both scope requires file and conversation precheck".to_owned();
        }

        let prepared_at = CreatedAt::now();
        let mut registry = self.plans();
        registry.entries.insert(
            plan.plan_id.clone(),
            PreparedRewind {
                plan: plan.clone(),
                prepared_at,
            },
        );
        registry.drop_stale(prepared_at);
        plan
    }

    pub fn validate_plan_session_revision(
        &self,
        plan_id: &str,
        current: i64,
    ) -> Result<(), RewindError> {
        self.validate_plan_session_revision_at(plan_id, current, CreatedAt::now())
    }

    pub(super) fn validate_plan_session_revision_at(
        &self,
        plan_id: &str,
        current: i64,
        now: CreatedAt,
    ) -> Result<(), RewindError> {
        let plan = self.plans().get(plan_id, now)?;
        if plan.session_revision == current {
            Ok(())
        } else {
            Err(RewindError::SessionRevisionMismatch)
        }
    }

    #[allow(dead_code)] // Transaction delivery consumes the prepared-plan seams.
    pub(crate) fn rewind_plan(&self, plan_id: &str) -> Result<RewindPlan, RewindError> {
        self.plans().get(plan_id, CreatedAt::now())
    }

    #[allow(dead_code)] // Transaction delivery consumes the prepared-plan seams.
    pub(crate) fn take_rewind_plan(&self, plan_id: &str) -> Result<RewindPlan, RewindError> {
        let mut registry = self.plans();
        let plan = registry.get(plan_id, CreatedAt::now())?;
        registry.entries.remove(plan_id);
        Ok(plan)
    }

    pub(super) fn earliest_revisions(&self, from_turn: i64) -> BTreeMap<String, FileRevision> {
        let state = self.state();
        let mut earliest: BTreeMap<String, FileRevision> = BTreeMap::new();
        for checkpoint in state.all() {
            if checkpoint.turn < from_turn {
                continue;
            }
            for revision in checkpoint.revisions() {
                let path = normalize_rel_path(&self.root, Path::new(&revision.path));
                if let Some(first) = earliest.get_mut(&path) {
                    first.after_existed = revision.after_existed;
                    first.after_sha256 = revision.after_sha256;
                    first.after_mode = revision.after_mode;
                } else {
                    let mut first = revision;
                    first.path = path.clone();
                    earliest.insert(path, first);
                }
            }
        }
        earliest
    }

    #[cfg(test)]
    pub(super) fn coverage_from_turn(
        &self,
        from_turn: i64,
    ) -> (Coverage, Vec<CoverageGap>, bool, bool) {
        let state = self.state();
        coverage_from_turn(&state.all(), from_turn)
    }

    pub(super) fn precheck_files(&self, from_turn: i64) -> Vec<RewindConflict> {
        let earliest = self.earliest_revisions(from_turn);
        let mut conflicts = Vec::new();
        for (path, revision) in earliest {
            let Ok(absolute) = safe_path(&self.root, Path::new(&path)) else {
                conflicts.push(RewindConflict {
                    path,
                    reason: CONFLICT_PATH_UNSAFE.to_owned(),
                    ..RewindConflict::default()
                });
                continue;
            };
            if revision.existed && revision.blob_ref.is_empty() && revision.content.is_none() {
                if !revision.sha256.is_empty()
                    && self.blobs.as_ref().is_some_and(|blobs| {
                        !blobs.has(&revision.sha256)
                            && (revision.blob_ref.is_empty() || !blobs.has(&revision.blob_ref))
                    })
                {
                    conflicts.push(RewindConflict {
                        path,
                        reason: CONFLICT_MISSING_PAYLOAD.to_owned(),
                        checkpoint_sha: revision.sha256,
                        ..RewindConflict::default()
                    });
                    continue;
                }
                if revision.blob_ref.is_empty() {
                    conflicts.push(RewindConflict {
                        path,
                        reason: CONFLICT_MISSING_PAYLOAD.to_owned(),
                        ..RewindConflict::default()
                    });
                    continue;
                }
            }
            let Ok(current) = fingerprint_path(&self.root, &absolute) else {
                conflicts.push(RewindConflict {
                    path,
                    reason: CONFLICT_EXTERNAL_CHANGE.to_owned(),
                    checkpoint_sha: revision.sha256,
                    ..RewindConflict::default()
                });
                continue;
            };
            let Some(reason) = compare_identity(
                &current,
                &revision.after_sha256,
                revision.after_existed,
                revision.after_mode,
            ) else {
                continue;
            };
            if reason == CONFLICT_COVERAGE_LEGACY {
                continue;
            }
            conflicts.push(RewindConflict {
                path,
                reason: reason.to_owned(),
                checkpoint_sha: revision.sha256,
                last_owned_sha: revision.after_sha256,
                current_sha: current.sha256,
                checkpoint_mode: revision.mode,
                current_mode: current.mode,
                current_existed: current.existed,
                checkpoint_existed: revision.existed,
            });
        }
        conflicts.sort_by(|left, right| left.path.cmp(&right.path));
        conflicts
    }

    fn plans(&self) -> MutexGuard<'_, PlanRegistry> {
        self.runtime
            .plans
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

fn files_from_turn(root: &Path, checkpoints: &[Checkpoint], from_turn: i64) -> Vec<String> {
    let mut paths = BTreeSet::new();
    for checkpoint in checkpoints {
        if checkpoint.turn < from_turn {
            continue;
        }
        for revision in checkpoint.revisions() {
            paths.insert(normalize_rel_path(root, Path::new(&revision.path)));
        }
    }
    paths.into_iter().collect()
}

fn coverage_from_turn(
    checkpoints: &[Checkpoint],
    from_turn: i64,
) -> (Coverage, Vec<CoverageGap>, bool, bool) {
    let mut gaps = Vec::new();
    let mut legacy = false;
    let mut expired = false;
    let mut has_files = false;
    let mut partial = false;
    for checkpoint in checkpoints {
        if checkpoint.turn < from_turn {
            continue;
        }
        legacy |= checkpoint.schema_version < SCHEMA_V2
            || checkpoint.coverage == Coverage::Legacy
            || checkpoint.legacy;
        expired |= checkpoint.expired_file_payload;
        partial |= checkpoint.coverage == Coverage::Partial;
        gaps.extend(checkpoint.coverage_gaps.iter().cloned());
        has_files |= !checkpoint.revisions().is_empty();
    }
    if legacy {
        gaps.push(CoverageGap::new(
            GAP_LEGACY_UNVERIFIED,
            "legacy checkpoint unverified",
        ));
        return (Coverage::Legacy, gaps, true, expired);
    }
    if expired {
        gaps.push(CoverageGap::new(GAP_EXPIRED_PAYLOAD, EXPIRED_DETAIL));
        return (Coverage::Partial, gaps, false, true);
    }
    if !has_files {
        return (Coverage::None, gaps, false, false);
    }
    if partial || !gaps.is_empty() {
        (Coverage::Partial, gaps, false, false)
    } else {
        (Coverage::Complete, Vec::new(), false, false)
    }
}

fn new_plan_id(created_at: CreatedAt) -> String {
    let timestamp = created_at.get().timestamp_nanos_opt().unwrap_or_default();
    let suffix = digest(timestamp.to_string().as_bytes());
    format!("plan-{timestamp}-{}", &suffix[..8])
}
