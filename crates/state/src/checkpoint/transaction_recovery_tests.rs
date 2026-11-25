use std::fs;
use std::path::{Path, PathBuf};

use super::transaction_test_support::*;
use super::*;

fn prepared() -> (Fixture, String) {
    let fixture = modify_add_delete_fixture();
    let plan = plan(&fixture.store, RewindScope::Code);
    assert!(plan.can_files);
    (fixture, plan.plan_id)
}

fn crash_at(fixture: &Fixture, plan_id: &str, phase: TransactionPhase) {
    let injection = TransactionInjection::crash(phase);
    assert!(
        fixture
            .store
            .commit_rewind(plan_id, 7, b"forward", None, Some(&injection))
            .is_err()
    );
}

fn published_intent() -> Fixture {
    let (fixture, plan_id) = prepared();
    crash_at(
        &fixture,
        &plan_id,
        TransactionPhase::CrashAfterPublishedIntent,
    );
    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.state, TransactionState::Committing);
    assert!(durable.targets[0].published);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    fixture
}

fn restart(fixture: &Fixture) -> Store {
    Store::new(Some(&fixture.dir), fixture.root.path())
}

#[test]
fn startup_aborts_prepared_transaction_and_removes_publish_temp() {
    let (fixture, plan_id) = prepared();
    crash_at(&fixture, &plan_id, TransactionPhase::CrashAfterPrepared);
    let before = single_manifest(&fixture.dir).0;
    let temps: Vec<PathBuf> = before
        .targets
        .iter()
        .filter(|target| !target.publish_tmp.is_empty())
        .map(|target| PathBuf::from(&target.publish_tmp))
        .collect();
    assert!(!temps.is_empty());

    let recovered = restart(&fixture);
    let after = single_manifest(&fixture.dir).0;
    assert_eq!(after.state, TransactionState::Aborted);
    assert!(temps.iter().all(|path| !path.exists()));
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    assert!(!recovered.recovery_notes().is_empty());
}

#[test]
fn startup_compensates_durable_published_intent_before_rename() {
    let fixture = published_intent();
    let before = single_manifest(&fixture.dir).0;
    let temp = PathBuf::from(&before.targets[0].publish_tmp);
    assert!(temp.exists());
    assert!(!Path::new(&before.targets[0].backup_path).exists());

    let _recovered = restart(&fixture);
    let after = single_manifest(&fixture.dir).0;
    assert_eq!(after.state, TransactionState::Aborted);
    assert!(!temp.exists());
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
}

#[test]
fn startup_compensates_publish_before_progress_durable_manifest() {
    let (fixture, plan_id) = prepared();
    crash_at(
        &fixture,
        &plan_id,
        TransactionPhase::CrashAfterPublishBeforeProgress,
    );
    let before = single_manifest(&fixture.dir).0;
    assert_eq!(before.state, TransactionState::Committing);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
    assert!(Path::new(&before.targets[0].backup_path).exists());

    let _recovered = restart(&fixture);
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Aborted
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
}

#[test]
fn startup_preserves_external_edit_and_writes_forward_recovery_copy() {
    let fixture = published_intent();
    write(fixture.root.path(), "a.txt", "external-edit");
    let before = single_manifest(&fixture.dir).0;
    let target = &before.targets[0];
    let suffix = target.restore_sha.get(..8).unwrap_or("unknown");
    let recovery = PathBuf::from(format!("{}.reasonix-recovery-{suffix}", target.abs_path));

    let _recovered = restart(&fixture);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"external-edit");
    assert_eq!(fs::read(&recovery).unwrap(), b"a-after");
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Committing
    );
}

#[test]
fn failed_compensation_stays_committing_and_startup_retry_completes() {
    let (fixture, plan_id) = prepared();
    let injection = TransactionInjection::fail_after(TransactionPhase::PublishFile, 1)
        .and_fail(TransactionPhase::CompensateFile);
    let error = fixture
        .store
        .commit_rewind(&plan_id, 7, b"forward", None, Some(&injection))
        .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::CompensationPending { .. }
    ));
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Committing
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");

    let _recovered = restart(&fixture);
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Aborted
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
}

#[test]
fn startup_defers_conversation_scope_without_applier() {
    let fixture = published_intent();
    let mut durable = single_manifest(&fixture.dir).0;
    durable.scope = RewindScope::Conversation;
    save_manifest(&fixture.dir, &durable);
    let temp = PathBuf::from(&durable.targets[0].publish_tmp);
    assert!(temp.exists());

    let recovered = restart(&fixture);
    let after = single_manifest(&fixture.dir).0;
    assert_eq!(after.state, TransactionState::Committing);
    assert_eq!(after.scope, RewindScope::Conversation);
    assert!(temp.exists());
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert!(
        recovered
            .recovery_notes()
            .iter()
            .any(|note| note.phase == RecoveryPhase::DeferredConversation)
    );
}

#[test]
fn startup_aborts_prepared_conversation_transaction() {
    let (fixture, plan_id) = prepared();
    crash_at(&fixture, &plan_id, TransactionPhase::CrashAfterPrepared);
    let mut durable = single_manifest(&fixture.dir).0;
    durable.scope = RewindScope::Conversation;
    save_manifest(&fixture.dir, &durable);

    let _recovered = restart(&fixture);
    let after = single_manifest(&fixture.dir).0;
    assert_eq!(after.state, TransactionState::Aborted);
    assert_eq!(after.scope, RewindScope::Conversation);
    assert!(!after.targets.iter().any(|target| {
        !target.publish_tmp.is_empty() && Path::new(&target.publish_tmp).exists()
    }));
}

#[test]
fn startup_ignores_terminal_transaction_manifests() {
    for state in [
        TransactionState::Committed,
        TransactionState::Invalidated,
        TransactionState::Undone,
        TransactionState::Aborted,
    ] {
        let fixture = published_intent();
        let mut durable = single_manifest(&fixture.dir).0;
        durable.state = state;
        save_manifest(&fixture.dir, &durable);

        let _recovered = restart(&fixture);
        assert_eq!(single_manifest(&fixture.dir).0.state, state);
        assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
        assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    }
}

#[test]
fn unreadable_manifest_and_recovery_directory_produce_notes() {
    let fixture = fixture();
    fs::create_dir_all(transaction_dir(&fixture.dir)).unwrap();
    fs::write(transaction_dir(&fixture.dir).join("broken.json"), b"{").unwrap();
    let recovered = restart(&fixture);
    assert!(
        recovered
            .recovery_notes()
            .iter()
            .any(|note| note.phase == RecoveryPhase::ReadManifest)
    );

    let blocked = transaction_test_support::fixture();
    fs::create_dir_all(&blocked.dir).unwrap();
    fs::write(transaction_dir(&blocked.dir), b"not a directory").unwrap();
    let recovered = restart(&blocked);
    assert!(
        recovered
            .recovery_notes()
            .iter()
            .any(|note| note.phase == RecoveryPhase::ReadDirectory)
    );
}

#[test]
fn in_memory_store_does_not_scan_transaction_directory() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("transactions")).unwrap();
    fs::write(root.path().join("transactions").join("broken.json"), b"{").unwrap();
    let store = Store::new(None, root.path());
    assert!(store.recovery_notes().is_empty());
}

#[test]
fn startup_removes_new_file_only_while_it_matches_restore_state() {
    let fixture = published_intent();
    let mut durable = single_manifest(&fixture.dir).0;
    let index = durable
        .targets
        .iter()
        .position(|target| target.path == "c.txt")
        .unwrap();
    durable.targets[index].published = true;
    save_manifest(&fixture.dir, &durable);
    write(fixture.root.path(), "c.txt", "external-edit");

    let _recovered = restart(&fixture);
    assert_eq!(read(fixture.root.path(), "c.txt"), b"external-edit");
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Committing
    );

    write(fixture.root.path(), "c.txt", "c-before");
    let _recovered = restart(&fixture);
    assert!(!fixture.root.path().join("c.txt").exists());
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Aborted
    );
}
