//! ACP-038 behavior coverage: write-authority binding, bind-or-close cleanup,
//! recovery decision table, redirect sidecar backfill, and transcript-path
//! resolution fallbacks (`session_write_authority.go:16-151`,
//! `service.go:664-697`).

use reasonix_acp::authority::{
    AuthorityLease, LeaseAcquire, RecoveryInput, RecoveryOutcome, SnapshotKind, TargetView,
    WriteAuthority, basename, bind_authority, bind_or_close, build_redirect_meta, decide_recovery,
    resume_for_write, snapshot_kind, transcript_path,
};
use reasonix_acp::sidecar::AcpSessionMeta;

#[derive(Default)]
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
impl FakeCtrl {
    fn fake() -> Self {
        Self {
            concrete: false,
            bind_err: None,
            closed: false,
        }
    }
    fn concrete() -> Self {
        Self {
            concrete: true,
            bind_err: None,
            closed: false,
        }
    }
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
fn non_concrete_controller_stays_unbound_without_releasing_lease() {
    let mut lease = FakeLease::default();
    let mut ctrl = FakeCtrl::fake();
    let mut slot: Option<&mut dyn AuthorityLease> = Some(&mut lease);
    bind_authority(&mut ctrl, &mut slot).unwrap();
    assert!(!ctrl.closed);
    assert!(!lease.released);
}

#[test]
fn concrete_controller_binds_and_fake_does_not_snapshot_rewrite() {
    assert_eq!(snapshot_kind(true), SnapshotKind::Rewrite);
    assert_eq!(snapshot_kind(false), SnapshotKind::Plain);

    let mut lease = FakeLease::default();
    let mut ctrl = FakeCtrl::concrete();
    let mut slot: Option<&mut dyn AuthorityLease> = Some(&mut lease);
    bind_authority(&mut ctrl, &mut slot).unwrap();
    assert!(!ctrl.closed);
}

#[test]
fn bind_or_close_releases_lease_then_closes_on_bind_failure() {
    let mut lease = FakeLease::default();
    let mut ctrl = FakeCtrl {
        concrete: true,
        bind_err: Some("lease already held".to_owned()),
        closed: false,
    };
    let err = bind_or_close(&mut ctrl, Some(&mut lease)).unwrap_err();
    assert_eq!(err, "lease already held");
    assert!(lease.released);
    assert!(ctrl.closed);
}

#[test]
fn resume_refuses_before_bind_when_resume_fails() {
    let mut ctrl = FakeCtrl::concrete();
    let err =
        resume_for_write(&mut ctrl, None, Err("transcript not found".to_owned())).unwrap_err();
    assert_eq!(err, "transcript not found");
    assert!(!ctrl.closed);
}

#[test]
fn recovery_noop_when_path_empty_or_session_gone() {
    let empty = RecoveryInput {
        recovery_path: "   ",
        session_found: true,
        lease: LeaseAcquire::Acquired,
        deleted: false,
        bind_ok: true,
    };
    assert_eq!(decide_recovery(&empty), RecoveryOutcome::NoOp);

    let gone = RecoveryInput {
        recovery_path: "/dir/1.jsonl",
        session_found: false,
        lease: LeaseAcquire::Acquired,
        deleted: false,
        bind_ok: true,
    };
    assert_eq!(decide_recovery(&gone), RecoveryOutcome::NoOp);
}

#[test]
fn recovery_held_lease_message_names_holder_and_close_hint() {
    let input = RecoveryInput {
        recovery_path: "/dir/1.jsonl",
        session_found: true,
        lease: LeaseAcquire::Held {
            holder: "another Reasonix window".to_owned(),
        },
        deleted: false,
        bind_ok: true,
    };
    let RecoveryOutcome::LeaseHeld { message } = decide_recovery(&input) else {
        panic!("held lease must win");
    };
    assert_eq!(
        message,
        "bind recovery session: another Reasonix window; close the other Reasonix window or process first"
    );
}

#[test]
fn recovery_other_lease_wraps_detail_without_close_hint() {
    let input = RecoveryInput {
        recovery_path: "/dir/1.jsonl",
        session_found: true,
        lease: LeaseAcquire::Other {
            detail: "lock file unreadable".to_owned(),
        },
        deleted: false,
        bind_ok: true,
    };
    let RecoveryOutcome::LeaseOther { message } = decide_recovery(&input) else {
        panic!("other lease must win");
    };
    assert_eq!(message, "bind recovery session: lock file unreadable");
}

#[test]
fn recovery_acquired_branch_deleted_bind_failed_and_moved() {
    let base = RecoveryInput {
        recovery_path: "/dir/1.jsonl",
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

    let failed = RecoveryInput {
        bind_ok: false,
        ..base.clone()
    };
    assert_eq!(decide_recovery(&failed), RecoveryOutcome::BindFailed);
}

#[test]
fn redirect_sidecar_backfills_identity_and_keeps_basename() {
    let live = AcpSessionMeta {
        session_id: String::new(),
        cwd: "/proj".to_owned(),
        created_at: "2026-11-24T08:00:00Z".to_owned(),
        ..Default::default()
    };
    let meta = build_redirect_meta("sess-9", "/proj/.reasonix/1/rec-3.jsonl", &live);
    assert_eq!(meta.session_id, "sess-9");
    assert_eq!(meta.cwd, "/proj");
    assert_eq!(meta.created_at, "2026-11-24T08:00:00Z");
    assert_eq!(meta.active_transcript.as_deref(), Some("rec-3.jsonl"));
    assert_eq!(basename("/proj/.reasonix/1/rec-3.jsonl"), "rec-3.jsonl");
}

#[test]
fn transcript_path_joins_id_with_jsonl() {
    let expected = std::path::Path::new("/proj/.reasonix/1")
        .join("sess-9.jsonl")
        .to_string_lossy()
        .into_owned();
    assert_eq!(transcript_path("/proj/.reasonix/1", "sess-9"), expected);
}

#[test]
fn resolution_prefers_id_path_on_every_invalid_redirect() {
    let dir = "/proj/.reasonix/1";
    let id = "sess-9";
    let id_path = transcript_path(dir, id);

    let ok = TargetView {
        exists: true,
        session_id: Some(id.to_owned()),
    };
    let active = reasonix_acp::authority::RedirectMetaView {
        active_transcript: Some("rec-3.jsonl".to_owned()),
    };
    assert_eq!(
        reasonix_acp::authority::resolve_transcript_path(dir, id, Some(&active), Some(&ok)),
        std::path::Path::new(dir)
            .join("rec-3.jsonl")
            .to_string_lossy()
            .into_owned()
    );

    let missing_target = TargetView {
        exists: false,
        session_id: None,
    };
    assert_eq!(
        reasonix_acp::authority::resolve_transcript_path(
            dir,
            id,
            Some(&active),
            Some(&missing_target)
        ),
        id_path
    );

    let wrong_id = TargetView {
        exists: true,
        session_id: Some("other".to_owned()),
    };
    assert_eq!(
        reasonix_acp::authority::resolve_transcript_path(dir, id, Some(&active), Some(&wrong_id)),
        id_path
    );

    let nested = reasonix_acp::authority::RedirectMetaView {
        active_transcript: Some("../escape.jsonl".to_owned()),
    };
    assert_eq!(
        reasonix_acp::authority::resolve_transcript_path(dir, id, Some(&nested), Some(&ok)),
        id_path
    );

    let empty = reasonix_acp::authority::RedirectMetaView {
        active_transcript: None,
    };
    assert_eq!(
        reasonix_acp::authority::resolve_transcript_path(dir, id, Some(&empty), Some(&ok)),
        id_path
    );
    assert_eq!(
        reasonix_acp::authority::resolve_transcript_path(dir, id, None, Some(&ok)),
        id_path
    );
}
