use std::fs;
use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;

use super::transaction_test_support::*;
use super::*;

fn assert_recovery_blocks_mutation(store: &Store) {
    assert!(matches!(
        store.invalidate_undo(),
        Err(StoreError::UndoRecoveryPending)
    ));
    assert!(matches!(
        store.begin(8, "blocked", 8),
        Err(StoreError::UndoRecoveryPending)
    ));
    let observer = MutationObserver::new(ObserverOptions {
        store,
        ownership_turn: 0,
        writer_id: "blocked".to_owned(),
        background: false,
        seq: None,
    });
    assert!(matches!(
        observer.try_begin_write(&CancellationToken::new()),
        Err(BeginWriteError::Undo(StoreError::UndoRecoveryPending))
    ));
}

fn assert_recovery_blocks_rewind(store: &Store) {
    let rewind = plan(store, RewindScope::Conversation);
    assert!(rewind.can_conversation, "{rewind:?}");
    assert!(matches!(
        store.commit_rewind(&rewind.plan_id, 7, b"forward", None, None),
        Err(TransactionError::Store(StoreError::UndoRecoveryPending))
    ));
}

fn assert_recovery_allows_mutation(store: &Store) {
    store.invalidate_undo().unwrap();
    store.begin(8, "recovered", 8).unwrap();
}

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

    let recovered = restart(&fixture);
    let after = single_manifest(&fixture.dir).0;
    assert_eq!(after.state, TransactionState::Aborted);
    assert!(!temp.exists());
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    assert_recovery_allows_mutation(&recovered);
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

    let recovered = restart(&fixture);
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Aborted
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    assert_recovery_allows_mutation(&recovered);
}

#[test]
fn startup_preserves_external_edit_and_writes_forward_recovery_copy() {
    let fixture = published_intent();
    write(fixture.root.path(), "a.txt", "external-edit");
    let before = single_manifest(&fixture.dir).0;
    let target = &before.targets[0];
    let suffix = target.restore_sha.get(..8).unwrap_or("unknown");
    let recovery = PathBuf::from(format!("{}.reasonix-recovery-{suffix}", target.abs_path));

    let recovered = restart(&fixture);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"external-edit");
    assert_eq!(fs::read(&recovery).unwrap(), b"a-after");
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Committing
    );
    assert_recovery_blocks_mutation(&recovered);
    assert_recovery_blocks_rewind(&recovered);
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
    assert_recovery_blocks_mutation(&fixture.store);
    assert_recovery_blocks_rewind(&fixture.store);

    let recovered = restart(&fixture);
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Aborted
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    assert_recovery_allows_mutation(&recovered);
}

#[test]
#[cfg(windows)]
fn startup_compensation_with_unwritable_abort_manifest_stays_pending() {
    let fixture = published_intent();
    let (before, manifest) = single_manifest(&fixture.dir);
    let temp = PathBuf::from(&before.targets[0].publish_tmp);
    let mut permissions = fs::metadata(&manifest).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&manifest, permissions).unwrap();

    let recovered = restart(&fixture);
    assert!(!temp.exists());
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Committing
    );
    assert!(
        recovered
            .recovery_notes()
            .iter()
            .any(|note| note.phase == RecoveryPhase::PersistAborted)
    );
    assert_recovery_blocks_mutation(&recovered);
    assert_recovery_blocks_rewind(&recovered);

    reasonix_core::fileutil::set_file_mode(&manifest, 0o600).unwrap();
    let recovered = restart(&fixture);
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Aborted
    );
    assert_recovery_allows_mutation(&recovered);
}

#[cfg(windows)]
struct ReadOnlyManifestApplier<'a> {
    dir: &'a Path,
}

#[cfg(windows)]
impl ConversationApplier for ReadOnlyManifestApplier<'_> {
    fn apply_conversation_truncate(
        &self,
        _boundary: i64,
        _forward: &[u8],
    ) -> Result<(), ConversationError> {
        Ok(())
    }

    fn restore_conversation(&self, _forward: &[u8]) -> Result<(), ConversationError> {
        Ok(())
    }

    fn truncate_checkpoints(&self, _from_turn: i64) -> Result<(), ConversationError> {
        let (_, manifest) = single_manifest(self.dir);
        reasonix_core::fileutil::set_file_mode(&manifest, 0o400).unwrap();
        Ok(())
    }

    fn restore_checkpoints(&self, _backup: &[u8]) -> Result<(), ConversationError> {
        Ok(())
    }
}

#[test]
#[cfg(windows)]
fn live_compensation_with_unwritable_abort_manifest_stays_pending() {
    let fixture = modify_add_delete_fixture();
    let plan = fixture
        .store
        .prepare_rewind(0, RewindScope::Both, 7, 5, true);
    assert!(plan.can_files && plan.can_conversation, "{plan:?}");
    let applier = ReadOnlyManifestApplier { dir: &fixture.dir };

    let error = fixture
        .store
        .commit_rewind(
            &plan.plan_id,
            7,
            b"conversation-forward",
            Some(&applier),
            None,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        TransactionError::CompensationPending { .. }
    ));
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    let (durable, manifest) = single_manifest(&fixture.dir);
    assert_eq!(durable.state, TransactionState::Committing);
    assert_recovery_blocks_mutation(&fixture.store);
    assert_recovery_blocks_rewind(&fixture.store);

    reasonix_core::fileutil::set_file_mode(&manifest, 0o600).unwrap();
    let recovered = restart(&fixture);
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Committing
    );
    assert!(
        recovered
            .recovery_notes()
            .iter()
            .any(|note| note.phase == RecoveryPhase::DeferredConversation)
    );
    assert_recovery_blocks_mutation(&recovered);
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
    assert_recovery_blocks_mutation(&recovered);
    assert_recovery_blocks_rewind(&recovered);
}

#[test]
fn startup_defers_unsupported_transaction_kind() {
    let fixture = published_intent();
    let mut durable = single_manifest(&fixture.dir).0;
    durable.kind = "unsupported".to_owned();
    save_manifest(&fixture.dir, &durable);

    let recovered = restart(&fixture);
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Committing
    );
    assert!(
        recovered
            .recovery_notes()
            .iter()
            .any(|note| note.phase == RecoveryPhase::UnsupportedKind)
    );
    assert_recovery_blocks_mutation(&recovered);
    assert_recovery_blocks_rewind(&recovered);
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
fn startup_restores_committed_file_revert_offer_like_go() {
    let fixture = published_intent();
    let mut durable = single_manifest(&fixture.dir).0;
    durable.kind = "file_revert".to_owned();
    durable.state = TransactionState::Committed;
    save_manifest(&fixture.dir, &durable);

    let recovered = restart(&fixture);

    assert_eq!(recovered.current_undo_offer().unwrap().id, durable.id);
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Committed
    );
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
    assert_recovery_blocks_mutation(&recovered);

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
    assert_recovery_blocks_mutation(&recovered);
}

#[test]
fn startup_fails_closed_for_dangling_transaction_directory_links() {
    let fixture = fixture();
    fs::create_dir_all(&fixture.dir).unwrap();
    let directory = transaction_dir(&fixture.dir);
    #[cfg(unix)]
    std::os::unix::fs::symlink("missing", &directory).unwrap();
    #[cfg(windows)]
    {
        let target = fixture.dir.join("missing");
        fs::create_dir(&target).unwrap();
        let comspec = std::env::var_os("COMSPEC").expect("Windows COMSPEC is required");
        let system_root = std::env::var_os("SystemRoot").expect("Windows SystemRoot is required");
        let output = std::process::Command::new(comspec)
            .env("SystemRoot", system_root)
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&directory)
            .arg(&target)
            .output()
            .unwrap();
        assert!(output.status.success(), "mklink /J failed: {output:?}");
        fs::remove_dir(target).unwrap();
    }

    let recovered = restart(&fixture);
    assert!(
        recovered
            .recovery_notes()
            .iter()
            .any(|note| note.phase == RecoveryPhase::ReadDirectory)
    );
    assert_recovery_blocks_mutation(&recovered);
    assert_recovery_blocks_rewind(&recovered);
}

#[test]
fn startup_fails_closed_for_non_directory_manifest_symlinks() {
    let fixture = fixture();
    let directory = transaction_dir(&fixture.dir);
    fs::create_dir_all(&directory).unwrap();
    let link = directory.join("linked.json");
    #[cfg(unix)]
    std::os::unix::fs::symlink("missing.json", &link).unwrap();
    #[cfg(windows)]
    {
        let target = fixture.dir.join("manifest-target");
        fs::create_dir(&target).unwrap();
        let comspec = std::env::var_os("COMSPEC").expect("Windows COMSPEC is required");
        let system_root = std::env::var_os("SystemRoot").expect("Windows SystemRoot is required");
        let output = std::process::Command::new(comspec)
            .env("SystemRoot", system_root)
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap();
        assert!(output.status.success(), "mklink /J failed: {output:?}");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    let recovered = restart(&fixture);
    assert!(
        recovered
            .recovery_notes()
            .iter()
            .any(|note| note.phase == RecoveryPhase::ReadManifest)
    );
    assert_recovery_blocks_mutation(&recovered);
    assert_recovery_blocks_rewind(&recovered);
}

#[test]
fn startup_ignores_directories_named_as_manifests() {
    let fixture = fixture();
    fs::create_dir_all(transaction_dir(&fixture.dir).join("directory.json")).unwrap();

    let recovered = restart(&fixture);
    assert!(recovered.recovery_notes().is_empty());
    recovered.invalidate_undo().unwrap();
    recovered.begin(8, "recovered", 8).unwrap();
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
    let recovered = restart(&fixture);
    assert!(!fixture.root.path().join("c.txt").exists());
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Aborted
    );
    assert_recovery_allows_mutation(&recovered);
}
