use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::sessionstore::eventlog::record::CreatedAt;

use super::transaction_test_support::*;
use super::*;

fn commit(
    fixture: &Fixture,
    plan_id: &str,
    forward: &[u8],
) -> Result<RewindResult, TransactionError> {
    fixture.store.commit_rewind(plan_id, 7, forward, None, None)
}

fn prepared_fixture() -> (Fixture, String) {
    let fixture = modify_add_delete_fixture();
    let plan = plan(&fixture.store, RewindScope::Code);
    assert!(plan.can_files);
    (fixture, plan.plan_id)
}

#[test]
fn commit_rewind_modify_add_delete_persists_progress_and_cleans_staging() {
    let (fixture, plan_id) = prepared_fixture();
    let result = commit(&fixture, &plan_id, b"conversation-forward").unwrap();

    assert!(result.ok);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
    assert_eq!(read(fixture.root.path(), "c.txt"), b"c-before");
    assert!(!fixture.root.path().join("b.txt").exists());
    assert_eq!(result.files.len(), 3);
    assert!(result.error.is_empty());

    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.id, result.transaction_id);
    assert_eq!(durable.state, TransactionState::Committed);
    assert!(durable.targets.iter().all(|target| target.published));
    assert!(durable.error.is_empty());
    assert!(!durable.targets.iter().any(|target| {
        !target.publish_tmp.is_empty() && Path::new(&target.publish_tmp).exists()
    }));
    assert!(
        !durable
            .targets
            .iter()
            .filter_map(|target| (!target.backup_path.is_empty()).then_some(&target.backup_path))
            .any(|path| Path::new(path).exists())
    );
    let error = commit(&fixture, &plan_id, b"conversation-forward").unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Rewind(RewindError::UnknownOrExpiredPlan)
    ));
}

#[test]
fn crash_after_prepared_leaves_targets_unchanged_and_temp_durable() {
    let (fixture, plan_id) = prepared_fixture();
    let injection = TransactionInjection::crash(TransactionPhase::CrashAfterPrepared);
    let error = fixture
        .store
        .commit_rewind(&plan_id, 7, b"conversation-forward", None, Some(&injection))
        .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::InjectedCrash {
            phase: TransactionPhase::CrashAfterPrepared
        }
    ));

    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.state, TransactionState::Prepared);
    assert!(durable.targets.iter().all(|target| !target.published));
    assert!(
        durable
            .targets
            .iter()
            .filter(|target| !target.publish_tmp.is_empty())
            .all(|target| Path::new(&target.publish_tmp).exists())
    );
    assert!(
        durable
            .targets
            .iter()
            .filter(|target| !target.publish_tmp.is_empty())
            .any(|target| fs::metadata(&target.publish_tmp).unwrap().len() > 0)
    );
    assert!(
        !durable
            .targets
            .iter()
            .filter_map(|target| (!target.backup_path.is_empty()).then_some(&target.backup_path))
            .any(|path| Path::new(path).exists())
    );
}

#[test]
fn missing_restore_blob_rejects_before_target_mutation() {
    let (mut fixture, plan_id) = prepared_fixture();
    fixture.store.config.retain = 1;
    fixture.store.begin(8, "", 0).unwrap();
    write(fixture.root.path(), "a.txt", "a-before");
    fixture
        .store
        .capture_before("a.txt", CaptureBeforeOpts::default());
    write(fixture.root.path(), "a.txt", "a-after");
    fixture
        .store
        .capture_after("a.txt", CaptureAfterOpts::default())
        .unwrap();
    fixture.store.begin(9, "", 0).unwrap();
    assert!(
        read_json_file::<Checkpoint>(&turn_path(&fixture.dir, 0, true))
            .unwrap()
            .expired_file_payload
    );
    assert!(fixture.store.blobs().unwrap().has(&digest(b"a-before")));

    let error = commit(&fixture, &plan_id, b"forward").unwrap_err();
    match error {
        TransactionError::PrecheckConflicts(conflicts) => {
            let conflict = conflicts
                .iter()
                .find(|conflict| conflict.path == "a.txt")
                .expect("missing restore payload conflict");
            assert_eq!(conflict.reason, CONFLICT_MISSING_PAYLOAD);
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    assert!(!fixture.root.path().join("c.txt").exists());
    assert_no_manifest(&fixture.dir);
    assert!(matches!(
        commit(&fixture, &plan_id, b"forward").unwrap_err(),
        TransactionError::Rewind(RewindError::UnknownOrExpiredPlan)
    ));
}

#[test]
fn publish_failure_compensates_published_files_and_aborts() {
    let (fixture, plan_id) = prepared_fixture();
    let injection = TransactionInjection::fail_after(TransactionPhase::PublishFile, 1);
    let error = fixture
        .store
        .commit_rewind(&plan_id, 7, b"conversation-forward", None, Some(&injection))
        .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Operation {
            phase: TransactionPhase::PublishFile,
            ..
        }
    ));

    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.state, TransactionState::Aborted);
    assert!(durable.targets[0].published);
    assert!(!durable.error.is_empty());
    assert!(!durable.targets.iter().any(|target| {
        !target.publish_tmp.is_empty() && Path::new(&target.publish_tmp).exists()
    }));
    assert!(
        !durable
            .targets
            .iter()
            .filter_map(|target| (!target.backup_path.is_empty()).then_some(&target.backup_path))
            .any(|path| Path::new(path).exists())
    );
}

#[test]
fn published_intent_persistence_failure_does_not_mark_target_published() {
    let (fixture, plan_id) = prepared_fixture();
    let injection = TransactionInjection::fail(TransactionPhase::PersistPublishedIntent);
    let error = fixture
        .store
        .commit_rewind(&plan_id, 7, b"conversation-forward", None, Some(&injection))
        .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Operation {
            phase: TransactionPhase::PersistPublishedIntent,
            ..
        }
    ));
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.state, TransactionState::Aborted);
    assert!(durable.targets.iter().all(|target| !target.published));
    assert!(!durable.targets.iter().any(|target| {
        !target.publish_tmp.is_empty() && Path::new(&target.publish_tmp).exists()
    }));
}

#[test]
fn session_revision_is_revalidated_and_plan_is_consumed() {
    let (fixture, plan_id) = prepared_fixture();
    let error = fixture
        .store
        .commit_rewind(&plan_id, 8, b"conversation-forward", None, None)
        .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Rewind(RewindError::SessionRevisionMismatch)
    ));
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    let error = commit(&fixture, &plan_id, b"conversation-forward").unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Rewind(RewindError::UnknownOrExpiredPlan)
    ));

    let plan = plan(&fixture.store, RewindScope::Code);
    let result = commit(&fixture, &plan.plan_id, b"conversation-forward").unwrap();
    assert!(result.ok);
    let error = commit(&fixture, &plan.plan_id, b"conversation-forward").unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Rewind(RewindError::UnknownOrExpiredPlan)
    ));
}

#[test]
fn workspace_token_is_revalidated_after_barrier_generation_changes() {
    let (fixture, plan_id) = prepared_fixture();
    let writer = fixture.store.barrier().try_enter_write().unwrap();
    drop(writer);
    let error = commit(&fixture, &plan_id, b"conversation-forward").unwrap_err();
    assert!(matches!(
        error,
        TransactionError::WorkspaceTokenMismatch { .. }
    ));
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
}

#[test]
fn active_writer_is_revalidated_before_prepare() {
    let (fixture, plan_id) = prepared_fixture();
    fixture.store.set_active_writers(vec![active_writer()]);
    let error = commit(&fixture, &plan_id, b"conversation-forward").unwrap_err();
    assert!(matches!(error, TransactionError::ActiveWriters));
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_no_manifest(&fixture.dir);
    assert!(matches!(
        commit(&fixture, &plan_id, b"conversation-forward").unwrap_err(),
        TransactionError::Rewind(RewindError::UnknownOrExpiredPlan)
    ));

    fixture.store.set_active_writers(Vec::new());
    let plan = plan(&fixture.store, RewindScope::Code);
    assert!(
        commit(&fixture, &plan.plan_id, b"conversation-forward")
            .unwrap()
            .ok
    );
}

#[test]
fn precheck_is_rerun_after_prepare() {
    let (fixture, plan_id) = prepared_fixture();
    write(fixture.root.path(), "a.txt", "external-edit");
    let error = commit(&fixture, &plan_id, b"conversation-forward").unwrap_err();
    assert!(matches!(error, TransactionError::PrecheckConflicts { .. }));
    assert_eq!(read(fixture.root.path(), "a.txt"), b"external-edit");
    assert_no_manifest(&fixture.dir);
}

#[test]
fn busy_barrier_rejection_releases_no_permit() {
    let (fixture, plan_id) = prepared_fixture();
    let exclusive = fixture.store.barrier().try_enter_exclusive().unwrap();
    let started = Instant::now();
    let error = commit(&fixture, &plan_id, b"conversation-forward").unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(error, TransactionError::BarrierBusy));
    assert_no_manifest(&fixture.dir);
    assert!(matches!(
        commit(&fixture, &plan_id, b"conversation-forward").unwrap_err(),
        TransactionError::Rewind(RewindError::UnknownOrExpiredPlan)
    ));
    drop(exclusive);
    let plan = plan(&fixture.store, RewindScope::Code);
    assert!(
        commit(&fixture, &plan.plan_id, b"conversation-forward")
            .unwrap()
            .ok
    );
}

#[test]
fn commit_rewind_rejects_unknown_and_consumed_plans() {
    let (fixture, plan_id) = prepared_fixture();
    let error = commit(&fixture, "unknown", b"forward").unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Rewind(RewindError::UnknownOrExpiredPlan)
    ));
    assert!(commit(&fixture, &plan_id, b"forward").unwrap().ok);
    let error = commit(&fixture, &plan_id, b"forward").unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Rewind(RewindError::UnknownOrExpiredPlan)
    ));
}

#[test]
fn commit_rewind_rejects_expired_plan() {
    let (fixture, plan_id) = prepared_fixture();
    let current = plan(&fixture.store, RewindScope::Code);
    let now = CreatedAt::from_datetime(current.created_at.get() + chrono::Duration::minutes(11));
    let error = fixture
        .store
        .commit_rewind_at(&plan_id, 7, b"forward", None, None, now)
        .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Rewind(RewindError::UnknownOrExpiredPlan)
    ));
}

#[test]
fn inline_preimage_without_encoding_restores_current_gbk_bytes() {
    let mut fixture = fixture();
    let before = reasonix_core::fileencoding::encode("before €", Kind::Gb18030).unwrap();
    write_bytes(fixture.root.path(), "encoding.txt", &before);
    let checkpoints = [checkpoint(
        0,
        vec![FileSnap {
            content: Some("before €".into()),
            sha256: digest(&before),
            ..owned_file(
                "encoding.txt",
                None,
                Some(true),
                &digest(&before),
                live_mode(),
            )
        }],
    )];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);
    let plan = plan(&fixture.store, RewindScope::Code);
    assert!(plan.can_files);

    assert!(commit(&fixture, &plan.plan_id, b"forward").unwrap().ok);
    assert_eq!(read(fixture.root.path(), "encoding.txt"), before);
}

#[test]
fn inline_preimage_without_encoding_restores_current_utf16_bom_bytes() {
    let mut fixture = fixture();
    let before = reasonix_core::fileencoding::encode("before €", Kind::Utf16Le).unwrap();
    write_bytes(fixture.root.path(), "encoding.txt", &before);
    let checkpoints = [checkpoint(
        0,
        vec![FileSnap {
            content: Some("before €".into()),
            sha256: digest(&before),
            ..owned_file(
                "encoding.txt",
                None,
                Some(true),
                &digest(&before),
                live_mode(),
            )
        }],
    )];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);
    let plan = plan(&fixture.store, RewindScope::Code);
    assert!(plan.can_files);

    assert!(commit(&fixture, &plan.plan_id, b"forward").unwrap().ok);
    assert_eq!(read(fixture.root.path(), "encoding.txt"), before);
}

#[test]
fn blob_restore_uses_raw_content_addressed_payload() {
    let mut fixture = fixture();
    let restore = b"\xff\xferaw\x00payload";
    let blob = fixture.store.blobs().unwrap().put(restore).unwrap();
    write(fixture.root.path(), "a.txt", "after");
    let checkpoints = [checkpoint(
        0,
        vec![FileSnap {
            blob_ref: blob.clone(),
            sha256: blob,
            ..owned_file("a.txt", None, Some(true), &digest(b"after"), live_mode())
        }],
    )];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);
    let plan = plan(&fixture.store, RewindScope::Code);
    assert!(plan.can_files);
    assert!(commit(&fixture, &plan.plan_id, b"forward").unwrap().ok);
    assert_eq!(read(fixture.root.path(), "a.txt"), restore);
}

#[test]
fn publish_target_rejects_preexisting_backup() {
    let (fixture, plan_id) = prepared_fixture();
    let injection = TransactionInjection::crash(TransactionPhase::CrashAfterPrepared);
    assert!(
        fixture
            .store
            .commit_rewind(&plan_id, 7, b"forward", None, Some(&injection))
            .is_err()
    );
    let durable = single_manifest(&fixture.dir).0;
    let target = durable.targets[0].clone();
    write_bytes(
        fixture.root.path(),
        &target.backup_path,
        b"unrelated-backup",
    );
    let error = transaction::publish_target(&fixture.store, &target).unwrap_err();
    assert!(matches!(
        error,
        TransactionError::BackupAlreadyExists { .. }
    ));
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(
        read(fixture.root.path(), &target.backup_path),
        b"unrelated-backup"
    );
}

#[test]
fn ownership_sync_updates_latest_after_identity_before_commit() {
    let (fixture, plan_id) = prepared_fixture();
    assert!(commit(&fixture, &plan_id, b"forward").unwrap().ok);
    let latest = read_json_file::<Checkpoint>(&turn_path(&fixture.dir, 0, false)).unwrap();
    let a = latest
        .files
        .iter()
        .find(|file| file.path == "a.txt")
        .unwrap();
    assert_eq!(a.after_sha256, digest(b"a-before"));
    assert_eq!(a.after_mode, live_mode());
    let b = latest
        .files
        .iter()
        .find(|file| file.path == "b.txt")
        .unwrap();
    assert_eq!(b.after_existed, Some(false));
    let c = latest
        .files
        .iter()
        .find(|file| file.path == "c.txt")
        .unwrap();
    assert_eq!(c.after_existed, Some(true));
    assert_eq!(c.after_sha256, digest(b"c-before"));
}
