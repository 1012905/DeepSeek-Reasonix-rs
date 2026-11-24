//! ACP-038 write-authority binding, recovery handler, and the id-keyed
//! redirect sidecar (`session_write_authority.go:16-151`, `service.go:664-697`).
//!
//! The concrete kernel controller generation-binds the session lease as write
//! authority; test fakes without the method stay unbound. A bind failure on
//! resume releases the lease and closes the controller so a half-bound
//! transcript cannot keep a stale handle. A conflict recovery at commit time
//! swaps the lease and transcript onto the recovery branch, retires the old
//! lease, and writes the single-hop id-keyed redirect sidecar. Real lease
//! acquisition, controller `Resume`/`Snapshot`/`SnapshotRewrite`, meta
//! persistence, and the retired-lease waiter are runtime wiring; this module
//! owns the bind-or-close cleanup order, the snapshot kind decision, the
//! recovery decision table, the redirect backfill, and the transcript-path
//! resolution fallback rules.

use crate::sidecar::AcpSessionMeta;

/// Lease seam handed to the controller as generation-bound write authority.
pub trait AuthorityLease {
    /// Releases the lease; called on bind failure before the controller closes.
    fn release(&mut self);
}

/// Controller write-authority seam.
pub trait WriteAuthority {
    /// True for the concrete kernel controller; fakes report `false` and remain
    /// unbound (`bindACPWriteAuthority` returns nil for non-concrete,
    /// `session_write_authority.go:17-20`).
    fn is_concrete(&self) -> bool;
    /// Binds the lease as write authority; only dispatched for concrete.
    fn bind_session_write_authority(
        &mut self,
        lease: &mut dyn AuthorityLease,
    ) -> Result<(), String>;
    /// Closes the controller after a failed bind (`bindOrClose`, line 29).
    fn close(&mut self);
}

/// Binds the write authority. Non-concrete controllers are a no-op success.
pub fn bind_authority(
    ctrl: &mut dyn WriteAuthority,
    lease: &mut Option<&mut dyn AuthorityLease>,
) -> Result<(), String> {
    if !ctrl.is_concrete() {
        return Ok(());
    }
    match lease.as_deref_mut() {
        Some(lease) => ctrl.bind_session_write_authority(lease),
        None => Ok(()),
    }
}

/// Bind, then on failure release the lease and close the controller
/// (`bindACPWriteAuthorityOrClose`, `session_write_authority.go:24-33`).
pub fn bind_or_close(
    ctrl: &mut dyn WriteAuthority,
    mut lease: Option<&mut dyn AuthorityLease>,
) -> Result<(), String> {
    if let Err(err) = bind_authority(ctrl, &mut lease) {
        if let Some(lease) = lease.take() {
            lease.release();
        }
        ctrl.close();
        return Err(err);
    }
    Ok(())
}

/// Resume a loaded transcript, then bind the write authority
/// (`resumeACPControllerForWrite`, lines 35-41). A resume error returns before
/// any bind; the in-flight turn owns the session it writes to.
pub fn resume_for_write(
    ctrl: &mut dyn WriteAuthority,
    mut lease: Option<&mut dyn AuthorityLease>,
    resume: Result<(), String>,
) -> Result<(), String> {
    resume?;
    bind_or_close(ctrl, lease.take())
}

/// Snapshot kind the controller exposes before a replacement swaps in
/// (`snapshotACPController`, lines 43-52).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    /// Concrete controller: rewrite snapshot.
    Rewrite,
    /// Fake/non-concrete: plain snapshot.
    Plain,
}

/// Selects the snapshot rewrite path for the concrete controller only.
pub fn snapshot_kind(concrete: bool) -> SnapshotKind {
    if concrete {
        SnapshotKind::Rewrite
    } else {
        SnapshotKind::Plain
    }
}

/// Joins a snapshot failure with the rollback error the replacement-prep
/// cleanup produced (`prepareACPReplacementAuthority`, lines 67-72):
/// `"<action>: <snapshot err>; <rollback err>"`.
pub fn join_authority_errors(
    action: &str,
    snapshot_err: &str,
    rollback_err: Option<&str>,
) -> String {
    match rollback_err {
        Some(rollback) => format!("{action}: {snapshot_err}; {rollback}"),
        None => format!("{action}: {snapshot_err}"),
    }
}

// ---- Recovery handler decision ------------------------------------------------

/// Lease acquisition outcome for the recovery transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseAcquire {
    /// Lease handed over for the swap.
    Acquired,
    /// Another holder owns the recovery transcript.
    Held { holder: String },
    /// Any other lease failure.
    Other { detail: String },
}

/// Input snapshot the recovery decision reads (`sessionRecoveredHandlerFor`,
/// lines 83-150).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryInput<'a> {
    /// Trimmed recovery transcript path.
    pub recovery_path: &'a str,
    /// Whether the ACP session this recovery belongs to still exists.
    pub session_found: bool,
    pub lease: LeaseAcquire,
    /// Session was deleted before the handler ran.
    pub deleted: bool,
    /// Whether binding the recovered transcript authority succeeded.
    pub bind_ok: bool,
}

/// Recovery handler outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// Empty path or missing session: no-op, no error.
    NoOp,
    /// Held lease: refuse with the in-use message plus close hint.
    LeaseHeld { message: String },
    /// Other lease failure: wrapped detail.
    LeaseOther { message: String },
    /// Deleted after acquisition: lease released, refuse.
    Deleted,
    /// Bind failure: old lease re-bound, new lease released, refuse.
    BindFailed,
    /// Lease + transcript swapped onto the recovery branch.
    Moved,
}

/// Close hint shared with the CLI and lease errors
/// (`control.SessionLeaseCloseHint`, lease keeper 261).
pub const LEASE_CLOSE_HINT: &str = "close the other Reasonix window or process first";

/// Replacement-controller bind failure (`session_write_authority.go:60-62`):
/// the error carries no underlying bind text.
pub const REPLACEMENT_BIND_FAILURE: &str = "bind replacement session authority";

/// Snapshot action label the reload path passes to the replacement-authority
/// prep (`service.go:816`, `"snapshot after reload"`).
pub const RELOAD_SNAPSHOT_ACTION: &str = "snapshot after reload";

/// Replacement bind failure message, verbatim and unwrapped.
pub fn replacement_bind_error() -> String {
    REPLACEMENT_BIND_FAILURE.to_owned()
}

/// Decides the recovery branch without touching disk or the store.
pub fn decide_recovery(input: &RecoveryInput) -> RecoveryOutcome {
    if input.recovery_path.trim().is_empty() || !input.session_found {
        return RecoveryOutcome::NoOp;
    }
    match &input.lease {
        LeaseAcquire::Held { holder } => {
            let holder = holder.trim();
            let holder = if holder.is_empty() {
                "this session is in use by another Reasonix window or process"
            } else {
                holder
            };
            RecoveryOutcome::LeaseHeld {
                message: format!("bind recovery session: {holder}; {LEASE_CLOSE_HINT}"),
            }
        }
        LeaseAcquire::Other { detail } => RecoveryOutcome::LeaseOther {
            message: format!("bind recovery session: {detail}"),
        },
        LeaseAcquire::Acquired => {
            if input.deleted {
                return RecoveryOutcome::Deleted;
            }
            if !input.bind_ok {
                return RecoveryOutcome::BindFailed;
            }
            RecoveryOutcome::Moved
        }
    }
}

/// Stable recovery error messages, matching the wrapped forms in
/// `session_write_authority.go:96-118`.
pub fn deleted_recovery_error() -> String {
    "bind recovery session: session is deleted".to_owned()
}

/// Bind-authority failure after lease acquisition.
pub fn bind_failed_recovery_error() -> String {
    "bind recovery session: unable to bind recovered transcript authority".to_owned()
}

// ---- Redirect sidecar and transcript resolution ---------------------------------

/// Transcript id-keyed path: `<dir>/<id>.jsonl` (`transcriptPath`, service.go:664-666).
pub fn transcript_path(dir: &str, id: &str) -> String {
    std::path::Path::new(dir)
        .join(format!("{id}.jsonl"))
        .to_string_lossy()
        .into_owned()
}

/// Basename aware of both `/` and `\` separators, mirroring `filepath.Base`.
pub fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed)
}

/// Id-keyed sidecar backfill when the session lives on a recovery branch
/// (`session_write_authority.go:128-147`): missing session id and cwd/createdAt
/// are backfilled from the live meta; `activeTranscript` becomes the recovery
/// transcript basename.
pub fn build_redirect_meta(id: &str, recovery_path: &str, live: &AcpSessionMeta) -> AcpSessionMeta {
    let mut meta = live.clone();
    if meta.session_id.trim().is_empty() {
        meta.session_id = id.to_owned();
    }
    if meta.cwd.trim().is_empty() {
        meta.cwd = live.cwd.clone();
    }
    if meta.created_at.trim().is_empty() {
        meta.created_at = live.created_at.clone();
    }
    meta.active_transcript = Some(basename(recovery_path).to_owned());
    meta
}

/// Id-keyed sidecar redirect view used by path resolution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RedirectMetaView {
    pub active_transcript: Option<String>,
}

/// Recovery-side target metadata view.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TargetView {
    pub exists: bool,
    pub session_id: Option<String>,
}

/// Resolves the transcript path the id-keyed sidecar points at
/// (`resolveTranscriptPath`, service.go:676-697). Every fallback — missing or
/// unreadable sidecar, empty/self redirect, basename with a directory, missing
/// target, or a target claiming a different session id — returns the id-keyed
/// path.
pub fn resolve_transcript_path(
    dir: &str,
    id: &str,
    id_meta: Option<&RedirectMetaView>,
    target: Option<&TargetView>,
) -> String {
    let id_path = transcript_path(dir, id);
    let Some(meta) = id_meta else {
        return id_path;
    };
    let active = meta.active_transcript.as_deref().unwrap_or("").trim();
    if active.is_empty() || active == basename(&id_path) {
        return id_path;
    }
    // A redirect carrying a separator is a path, not a basename: reject it.
    if active.contains('/') || active.contains('\\') {
        return id_path;
    }
    let Some(target) = target else {
        return id_path;
    };
    if !target.exists {
        return id_path;
    }
    let target_id = target.session_id.as_deref().unwrap_or("").trim();
    if target_id.is_empty() || target_id != id {
        return id_path;
    }
    std::path::Path::new(dir)
        .join(active)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeLease {
        released: bool,
    }
    impl AuthorityLease for FakeLease {
        fn release(&mut self) {
            self.released = true;
        }
    }

    struct FakeCtrl {
        concrete: bool,
        bind_err: Option<String>,
        closed: bool,
    }
    impl WriteAuthority for FakeCtrl {
        fn is_concrete(&self) -> bool {
            self.concrete
        }
        fn bind_session_write_authority(
            &mut self,
            _lease: &mut dyn AuthorityLease,
        ) -> Result<(), String> {
            self.bind_err.clone().map(Err).unwrap_or(Ok(()))
        }
        fn close(&mut self) {
            self.closed = true;
        }
    }

    #[test]
    fn non_concrete_fake_stays_unbound_without_error() {
        let mut ctrl = FakeCtrl {
            concrete: false,
            bind_err: None,
            closed: false,
        };
        assert!(bind_authority(&mut ctrl, &mut None).is_ok());
        assert!(!ctrl.closed);
    }

    #[test]
    fn bind_or_close_releases_lease_and_closes_controller_on_failure() {
        let mut lease = FakeLease { released: false };
        let mut ctrl = FakeCtrl {
            concrete: true,
            bind_err: Some("bind refused".to_owned()),
            closed: false,
        };
        let err = bind_or_close(&mut ctrl, Some(&mut lease)).unwrap_err();
        assert_eq!(err, "bind refused");
        assert!(lease.released);
        assert!(ctrl.closed);
    }

    #[test]
    fn resume_error_short_circles_before_bind() {
        let mut ctrl = FakeCtrl {
            concrete: true,
            bind_err: None,
            closed: false,
        };
        let err = resume_for_write(&mut ctrl, None, Err("load failed".to_owned())).unwrap_err();
        assert_eq!(err, "load failed");
        assert!(!ctrl.closed);
    }

    #[test]
    fn snapshot_kind_is_rewrite_only_for_concrete() {
        assert_eq!(snapshot_kind(true), SnapshotKind::Rewrite);
        assert_eq!(snapshot_kind(false), SnapshotKind::Plain);
    }

    #[test]
    fn replacement_bind_failure_is_verbatim_without_underlying_text() {
        assert_eq!(
            replacement_bind_error(),
            "bind replacement session authority"
        );
        assert_eq!(RELOAD_SNAPSHOT_ACTION, "snapshot after reload");
    }

    #[test]
    fn replacement_error_joins_action_snapshot_and_rollback() {
        assert_eq!(
            join_authority_errors(
                "prepare replacement",
                "snapshot conflict",
                Some("rebind failed")
            ),
            "prepare replacement: snapshot conflict; rebind failed"
        );
        assert_eq!(
            join_authority_errors("prepare replacement", "snapshot conflict", None),
            "prepare replacement: snapshot conflict"
        );
    }

    #[test]
    fn recovery_noop_on_empty_path_or_missing_session() {
        let empty = RecoveryInput {
            recovery_path: "  ",
            session_found: true,
            lease: LeaseAcquire::Acquired,
            deleted: false,
            bind_ok: true,
        };
        assert_eq!(decide_recovery(&empty), RecoveryOutcome::NoOp);
        let missing = RecoveryInput {
            recovery_path: "/x/1.jsonl",
            session_found: false,
            lease: LeaseAcquire::Acquired,
            deleted: false,
            bind_ok: true,
        };
        assert_eq!(decide_recovery(&missing), RecoveryOutcome::NoOp);
    }

    #[test]
    fn recovery_held_and_other_lease_outcomes_carry_verbatim_messages() {
        let held = RecoveryInput {
            recovery_path: "/x/1.jsonl",
            session_found: true,
            lease: LeaseAcquire::Held {
                holder: "another Reasonix window".to_owned(),
            },
            deleted: false,
            bind_ok: true,
        };
        let RecoveryOutcome::LeaseHeld { message } = decide_recovery(&held) else {
            panic!("held must win");
        };
        assert_eq!(
            message,
            "bind recovery session: another Reasonix window; close the other Reasonix window or process first"
        );

        let other = RecoveryInput {
            recovery_path: "/x/1.jsonl",
            session_found: true,
            lease: LeaseAcquire::Other {
                detail: "lock corrupted".to_owned(),
            },
            deleted: false,
            bind_ok: true,
        };
        let RecoveryOutcome::LeaseOther { message } = decide_recovery(&other) else {
            panic!("other must win");
        };
        assert_eq!(message, "bind recovery session: lock corrupted");
    }

    #[test]
    fn recovery_acquired_deleted_and_bind_fail_and_move() {
        let base = RecoveryInput {
            recovery_path: "/x/1.jsonl",
            session_found: true,
            lease: LeaseAcquire::Acquired,
            deleted: false,
            bind_ok: true,
        };
        assert_eq!(decide_recovery(&base), RecoveryOutcome::Moved);

        let deleted = RecoveryInput {
            deleted: true,
            ..base.clone()
        };
        assert_eq!(decide_recovery(&deleted), RecoveryOutcome::Deleted);
        assert_eq!(
            deleted_recovery_error(),
            "bind recovery session: session is deleted"
        );

        let failed = RecoveryInput {
            bind_ok: false,
            ..base.clone()
        };
        assert_eq!(decide_recovery(&failed), RecoveryOutcome::BindFailed);
        assert_eq!(
            bind_failed_recovery_error(),
            "bind recovery session: unable to bind recovered transcript authority"
        );
    }

    #[test]
    fn redirect_meta_backfills_id_cwd_created_at_and_basename() {
        let live = AcpSessionMeta {
            session_id: String::new(),
            cwd: "/proj".to_owned(),
            created_at: "2026-11-24T00:00:00Z".to_owned(),
            ..Default::default()
        };
        let meta = build_redirect_meta("sess-1", "/proj/.reasonix/1/rec-7.jsonl", &live);
        assert_eq!(meta.session_id, "sess-1");
        assert_eq!(meta.cwd, "/proj");
        assert_eq!(meta.created_at, "2026-11-24T00:00:00Z");
        assert_eq!(meta.active_transcript.as_deref(), Some("rec-7.jsonl"));
    }

    #[test]
    fn resolution_falls_back_on_every_bad_redirect_shape() {
        let dir = "/proj/.reasonix/1";
        let id = "sess-1";
        let id_path = transcript_path(dir, id);

        assert_eq!(resolve_transcript_path(dir, id, None, None), id_path);
        let empty = RedirectMetaView {
            active_transcript: None,
        };
        assert_eq!(
            resolve_transcript_path(dir, id, Some(&empty), Some(&TargetView::default())),
            id_path
        );
        let self_ref = RedirectMetaView {
            active_transcript: Some(format!("{id}.jsonl")),
        };
        assert_eq!(
            resolve_transcript_path(dir, id, Some(&self_ref), Some(&TargetView::default())),
            id_path
        );
        let path_ref = RedirectMetaView {
            active_transcript: Some("../outside.jsonl".to_owned()),
        };
        assert_eq!(
            resolve_transcript_path(dir, id, Some(&path_ref), Some(&TargetView::default())),
            id_path
        );
        let missing_target = RedirectMetaView {
            active_transcript: Some("rec-7.jsonl".to_owned()),
        };
        assert_eq!(
            resolve_transcript_path(dir, id, Some(&missing_target), None),
            id_path
        );
        let wrong_id = TargetView {
            exists: true,
            session_id: Some("other".to_owned()),
        };
        assert_eq!(
            resolve_transcript_path(dir, id, Some(&missing_target), Some(&wrong_id)),
            id_path
        );
        let ok = TargetView {
            exists: true,
            session_id: Some(id.to_owned()),
        };
        assert_eq!(
            resolve_transcript_path(dir, id, Some(&missing_target), Some(&ok)),
            std::path::Path::new(dir)
                .join("rec-7.jsonl")
                .to_string_lossy()
                .into_owned()
        );
    }
}
