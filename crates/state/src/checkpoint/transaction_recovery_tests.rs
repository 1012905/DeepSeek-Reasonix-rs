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
fn external_modification_retry_keeps_identity_and_existing_recovery_copy() {
    let fixture = published_intent();
    write(fixture.root.path(), "a.txt", "external-edit");
    let before = single_manifest(&fixture.dir).0;
    let target = &before.targets[0];
    let suffix = target.restore_sha.get(..8).unwrap_or("unknown");
    let recovery = PathBuf::from(format!("{}.reasonix-recovery-{suffix}", target.abs_path));

    let recovered = restart(&fixture);
    assert_eq!(fs::read(&recovery).unwrap(), b"a-after");
    let pending = single_manifest(&fixture.dir).0;
    assert_eq!(pending.state, TransactionState::Committing);
    fs::write(&recovery, b"existing recovery copy").unwrap();

    let mut stages = Vec::new();
    let error = transaction_compensate::compensate(&recovered, &pending.targets, &mut stages, None)
        .unwrap_err();
    let TransactionError::Compensation(errors) = error else {
        panic!("expected compensation errors");
    };
    assert_eq!(errors.len(), 1);
    let TransactionError::ExternalModification {
        path,
        recovery: actual_recovery,
    } = &errors[0]
    else {
        panic!("expected external modification identity: {:?}", errors[0]);
    };
    assert_eq!(path, &target.path);
    assert_eq!(actual_recovery.as_ref(), Some(&recovery));
    assert_eq!(fs::read(&recovery).unwrap(), b"existing recovery copy");
    assert_eq!(read(fixture.root.path(), "a.txt"), b"external-edit");
    assert!(stages.iter().any(|stage| {
        stage.path == target.path
            && !stage.compensated
            && stage
                .comp_error
                .contains(&recovery.to_string_lossy().to_string())
    }));

    let retried = restart(&fixture);
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Committing
    );
    assert_eq!(fs::read(&recovery).unwrap(), b"existing recovery copy");
    assert!(retried.recovery_notes().iter().any(|note| {
        note.phase == RecoveryPhase::CompensationPending
            && note.message.contains(&format!("{recovery:?}"))
    }));
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

fn manifests(dir: &Path) -> Vec<(TransactionManifest, PathBuf)> {
    transaction_paths(dir)
        .into_iter()
        .map(|path| {
            let manifest = read_json_file(&path).unwrap();
            (manifest, path)
        })
        .collect()
}

fn crash_conversation_rewind() -> (Fixture, TransactionManifest) {
    let fixture = modify_add_delete_fixture();
    let plan = fixture
        .store
        .prepare_rewind(0, RewindScope::Both, 7, 5, true);
    assert!(plan.can_files && plan.can_conversation, "{plan:?}");
    let applier = MockApplier::default();
    let injection =
        TransactionInjection::crash(TransactionPhase::CrashAfterConversationBeforeFinalize);
    assert!(matches!(
        fixture.store.commit_rewind(
            &plan.plan_id,
            7,
            b"conversation-forward",
            Some(&applier),
            Some(&injection),
        ),
        Err(TransactionError::InjectedCrash {
            phase: TransactionPhase::CrashAfterConversationBeforeFinalize
        })
    ));
    let transaction = single_manifest(&fixture.dir).0;
    assert_eq!(transaction.state, TransactionState::Committing);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
    (fixture, transaction)
}

fn crash_conversation_undo() -> (Fixture, TransactionManifest, PathBuf) {
    let fixture = modify_add_delete_fixture();
    let plan = fixture
        .store
        .prepare_rewind(0, RewindScope::Both, 7, 5, true);
    assert!(plan.can_files && plan.can_conversation, "{plan:?}");
    let applier = MockApplier::default();
    fixture
        .store
        .commit_rewind(
            &plan.plan_id,
            7,
            b"conversation-forward",
            Some(&applier),
            None,
        )
        .unwrap();
    let original = single_manifest(&fixture.dir).0;
    let injection =
        TransactionInjection::crash(TransactionPhase::CrashAfterConversationBeforeFinalize);
    assert!(matches!(
        fixture
            .store
            .undo_rewind(&original.id, Some(&applier), Some(&injection),),
        Err(TransactionError::InjectedCrash {
            phase: TransactionPhase::CrashAfterConversationBeforeFinalize
        })
    ));
    let (undo, path) = manifests(&fixture.dir)
        .into_iter()
        .find(|(transaction, _)| transaction.kind == "undo")
        .expect("interrupted undo manifest");
    assert_eq!(undo.state, TransactionState::Committing);
    assert_eq!(undo.parent_transaction, original.id);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    (fixture, undo, path)
}

struct OrderedRecoveryApplier<'a> {
    root: &'a Path,
    expected_file: &'a [u8],
    observations: std::sync::atomic::AtomicUsize,
}

impl OrderedRecoveryApplier<'_> {
    fn observe(&self) {
        assert_eq!(
            fs::read(self.root.join("a.txt")).unwrap(),
            self.expected_file,
            "conversation recovery must precede file compensation"
        );
        self.observations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl ConversationApplier for OrderedRecoveryApplier<'_> {
    fn apply_conversation_truncate(
        &self,
        _boundary: i64,
        _forward: &[u8],
    ) -> Result<(), ConversationError> {
        self.observe();
        Ok(())
    }

    fn restore_conversation(&self, _forward: &[u8]) -> Result<(), ConversationError> {
        self.observe();
        Ok(())
    }

    fn truncate_checkpoints(&self, _from_turn: i64) -> Result<(), ConversationError> {
        self.observe();
        Ok(())
    }

    fn restore_checkpoints(&self, _backup: &[u8]) -> Result<(), ConversationError> {
        self.observe();
        Ok(())
    }
}

#[test]
fn recovery_restores_conversation_before_file_compensation() {
    let (fixture, transaction) = crash_conversation_rewind();
    let recovered = restart(&fixture);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
    assert!(
        recovered
            .recovery_notes()
            .iter()
            .any(|note| note.phase == RecoveryPhase::DeferredConversation)
    );
    let applier = OrderedRecoveryApplier {
        root: fixture.root.path(),
        expected_file: b"a-before",
        observations: std::sync::atomic::AtomicUsize::new(0),
    };

    let report = recovered.recover_transactions_with_applier(Some(&applier));

    assert!(report.errors.is_empty(), "{report:?}");
    assert_eq!(
        applier
            .observations
            .load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    assert!(!fixture.root.path().join("c.txt").exists());
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Aborted
    );
    assert!(report.notes.iter().any(|note| {
        note.phase == RecoveryPhase::Compensated
            && note.path.ends_with(format!("{}.json", transaction.id))
    }));
}

#[test]
fn recovery_reapplies_undo_rewind_before_file_compensation() {
    let (fixture, undo, _) = crash_conversation_undo();
    let recovered = restart(&fixture);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    let applier = OrderedRecoveryApplier {
        root: fixture.root.path(),
        expected_file: b"a-after",
        observations: std::sync::atomic::AtomicUsize::new(0),
    };

    let report = recovered.recover_transactions_with_applier(Some(&applier));

    assert!(report.errors.is_empty(), "{report:?}");
    assert_eq!(
        applier
            .observations
            .load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
    assert!(!fixture.root.path().join("b.txt").exists());
    assert_eq!(read(fixture.root.path(), "c.txt"), b"c-before");
    let (durable, _) = manifests(&fixture.dir)
        .into_iter()
        .find(|(transaction, _)| transaction.id == undo.id)
        .unwrap();
    assert_eq!(durable.state, TransactionState::Aborted);
    assert_eq!(durable.boundary_index, undo.boundary_index);
    assert_eq!(durable.conversation_forward, undo.conversation_forward);
    assert_eq!(durable.truncate_from, undo.truncate_from);
}

#[test]
fn recovery_refuses_missing_forward_conversation_payload() {
    let fixture = modify_add_delete_fixture();
    let plan = fixture
        .store
        .prepare_rewind(0, RewindScope::Both, 7, 5, true);
    assert!(plan.can_files && plan.can_conversation, "{plan:?}");
    let applier = MockApplier::default();
    let injection =
        TransactionInjection::crash(TransactionPhase::CrashAfterConversationBeforeFinalize);
    assert!(
        fixture
            .store
            .commit_rewind(&plan.plan_id, 7, b"", Some(&applier), Some(&injection))
            .is_err()
    );
    let transaction = single_manifest(&fixture.dir).0;
    assert!(transaction.has_boundary);
    assert!(transaction.conversation_forward.is_empty());
    let recovered = restart(&fixture);
    let applier = MockApplier::default();

    let report = recovered.recover_transactions_with_applier(Some(&applier));

    assert!(report.errors.iter().any(|error| matches!(
        error,
        RecoveryError::MissingForwardConversation { transaction_id }
            if transaction_id == &transaction.id
    )));
    assert!(applier.calls.lock().unwrap().is_empty());
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
    let pending = single_manifest(&fixture.dir).0;
    assert_eq!(pending.state, TransactionState::Committing);
    assert_eq!(
        pending.error,
        "crash recovery conversation compensation pending: missing forward conversation payload"
    );
    assert!(report.notes.iter().any(|note| {
        note.phase == RecoveryPhase::ConversationPending
            && note.path.ends_with(format!("{}.json", transaction.id))
    }));
    assert_recovery_blocks_mutation(&recovered);
}

#[test]
fn failed_conversation_recovery_stays_committing_and_retries() {
    let (fixture, transaction) = crash_conversation_rewind();
    let recovered = restart(&fixture);
    let applier = MockApplier::default();
    applier.fail_at(ConversationPhase::RestoreConversation);

    let failed = recovered.recover_transactions_with_applier(Some(&applier));

    assert!(!failed.errors.is_empty());
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Committing
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
    assert_recovery_blocks_mutation(&recovered);
    assert_recovery_blocks_rewind(&recovered);
    assert!(failed.notes.iter().any(|note| {
        note.phase == RecoveryPhase::ConversationPending
            && note.path.ends_with(format!("{}.json", transaction.id))
    }));

    applier.failures.lock().unwrap().clear();
    let retried = recovered.recover_transactions_with_applier(Some(&applier));

    assert!(retried.errors.is_empty(), "{retried:?}");
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Aborted
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_recovery_allows_mutation(&recovered);
    assert_eq!(
        applier
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call, ConversationCall::RestoreConversation(_)))
            .count(),
        2
    );
}

#[test]
fn startup_recovers_file_only_undo() {
    let (fixture, original) = committed_fixture();
    let injection =
        TransactionInjection::crash(TransactionPhase::CrashAfterConversationBeforeFinalize);
    assert!(matches!(
        fixture
            .store
            .undo_rewind(&original.transaction_id, None, Some(&injection),),
        Err(TransactionError::InjectedCrash {
            phase: TransactionPhase::CrashAfterConversationBeforeFinalize
        })
    ));

    let recovered = restart(&fixture);

    let (undo, _) = manifests(&fixture.dir)
        .into_iter()
        .find(|(transaction, _)| transaction.kind == "undo")
        .unwrap();
    assert_eq!(undo.state, TransactionState::Aborted);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
    assert!(!fixture.root.path().join("b.txt").exists());
    assert_eq!(read(fixture.root.path(), "c.txt"), b"c-before");
    assert_eq!(
        recovered.current_undo_offer().unwrap().id,
        original.transaction_id
    );
    assert_recovery_allows_mutation(&recovered);
}

#[test]
fn startup_recovers_file_only_file_revert() {
    let (fixture, plan_id) = prepared();
    crash_at(
        &fixture,
        &plan_id,
        TransactionPhase::CrashAfterPublishBeforeProgress,
    );
    let mut transaction = single_manifest(&fixture.dir).0;
    transaction.kind = "file_revert".to_owned();
    save_manifest(&fixture.dir, &transaction);

    let recovered = restart(&fixture);

    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Aborted
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    assert!(!fixture.root.path().join("c.txt").exists());
    assert_recovery_allows_mutation(&recovered);
}

#[test]
fn in_memory_pending_undo_recovers_with_applier() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "a.txt", "a-before");
    let store = std::sync::Arc::new(Store::new(None, root.path()));
    store.begin(0, "edit", 0).unwrap();
    store.capture_before(root.path().join("a.txt"), CaptureBeforeOpts::default());
    write(root.path(), "a.txt", "a-after");
    store
        .capture_after(
            root.path().join("a.txt"),
            CaptureAfterOpts {
                seq: 1,
                ..CaptureAfterOpts::default()
            },
        )
        .unwrap();
    let plan = store.prepare_rewind(0, RewindScope::Both, 7, 5, true);
    assert!(plan.can_files && plan.can_conversation, "{plan:?}");
    let applier = MockApplier::default();
    let original = store
        .commit_rewind(
            &plan.plan_id,
            7,
            b"conversation-forward",
            Some(&applier),
            None,
        )
        .unwrap();
    let offers = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let callback_offers = offers.clone();
    let callback_store = std::sync::Arc::clone(&store);
    store.on_undo_change(move || {
        callback_offers.lock().unwrap().push(
            callback_store
                .available_undo()
                .map(|offer| offer.transaction_id),
        );
    });
    let injection =
        TransactionInjection::crash(TransactionPhase::CrashAfterConversationBeforeFinalize);
    let error = store
        .undo_rewind(&original.transaction_id, Some(&applier), Some(&injection))
        .unwrap_err();
    assert!(
        matches!(
            error,
            TransactionError::InjectedCrash {
                phase: TransactionPhase::CrashAfterConversationBeforeFinalize
            }
        ),
        "{error:?}"
    );
    assert!(store.available_undo().is_none());
    assert_eq!(*offers.lock().unwrap(), vec![None]);
    let recovery = MockApplier::default();

    let report = store.recover_transactions_with_applier(Some(&recovery));

    assert!(report.errors.is_empty(), "{report:?}");
    assert_eq!(
        store.available_undo().unwrap().transaction_id,
        original.transaction_id
    );
    assert_eq!(
        *offers.lock().unwrap(),
        vec![None, Some(original.transaction_id.clone())]
    );
    assert_eq!(
        *recovery.calls.lock().unwrap(),
        vec![
            ConversationCall::ApplyConversationTruncate {
                boundary: 5,
                forward: b"conversation-forward".to_vec(),
            },
            ConversationCall::TruncateCheckpoints(0),
        ]
    );
    assert_eq!(read(root.path(), "a.txt"), b"a-before");
}

#[test]
fn recovery_retry_is_idempotent() {
    let (fixture, _) = crash_conversation_rewind();
    let recovered = restart(&fixture);
    let applier = MockApplier::default();
    let first = recovered.recover_transactions_with_applier(Some(&applier));
    assert!(first.errors.is_empty(), "{first:?}");
    let before = single_manifest(&fixture.dir).0;
    let calls = applier.calls.lock().unwrap().clone();

    let second = recovered.recover_transactions_with_applier(Some(&applier));

    assert!(second.errors.is_empty(), "{second:?}");
    assert!(second.notes.is_empty());
    assert_eq!(single_manifest(&fixture.dir).0, before);
    assert_eq!(*applier.calls.lock().unwrap(), calls);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    assert!(!fixture.root.path().join("c.txt").exists());
}

#[test]
fn undo_recovery_does_not_load_parent_manifest() {
    let (fixture, undo, _) = crash_conversation_undo();
    fs::remove_file(manifest_path(&fixture.dir, &undo.parent_transaction)).unwrap();
    let recovered = restart(&fixture);
    let applier = MockApplier::default();

    let report = recovered.recover_transactions_with_applier(Some(&applier));

    assert!(report.errors.is_empty(), "{report:?}");
    assert_eq!(
        *applier.calls.lock().unwrap(),
        vec![
            ConversationCall::ApplyConversationTruncate {
                boundary: undo.boundary_index,
                forward: undo.conversation_forward.clone(),
            },
            ConversationCall::TruncateCheckpoints(undo.truncate_from),
        ]
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
    let (durable, _) = manifests(&fixture.dir)
        .into_iter()
        .find(|(transaction, _)| transaction.id == undo.id)
        .unwrap();
    assert_eq!(durable.state, TransactionState::Aborted);
}

#[test]
fn recovery_does_not_fall_back_to_superseded_undo() {
    let (fixture, first) = committed_fixture();
    let plan = fixture
        .store
        .prepare_rewind(0, RewindScope::Conversation, 7, 5, true);
    assert!(plan.can_conversation, "{plan:?}");
    let applier = MockApplier::default();
    let second = fixture
        .store
        .commit_rewind(&plan.plan_id, 7, b"second-forward", Some(&applier), None)
        .unwrap();
    assert_ne!(first.transaction_id, second.transaction_id);
    let plan = fixture
        .store
        .prepare_rewind(0, RewindScope::Conversation, 7, 5, true);
    assert!(plan.can_conversation, "{plan:?}");
    let injection =
        TransactionInjection::crash(TransactionPhase::CrashAfterConversationBeforeFinalize);
    assert!(
        fixture
            .store
            .commit_rewind(
                &plan.plan_id,
                7,
                b"third-forward",
                Some(&applier),
                Some(&injection),
            )
            .is_err()
    );
    let recovered = restart(&fixture);
    assert!(recovered.available_undo().is_none());
    let recovery = MockApplier::default();
    recovery.fail_at(ConversationPhase::RestoreConversation);

    let failed = recovered.recover_transactions_with_applier(Some(&recovery));

    assert!(!failed.errors.is_empty());
    assert!(recovered.available_undo().is_none());
    assert!(matches!(
        recovered.undo_rewind(&first.transaction_id, None, None),
        Err(TransactionError::Store(StoreError::UndoRecoveryPending))
    ));
    recovery.failures.lock().unwrap().clear();
    let retried = recovered.recover_transactions_with_applier(Some(&recovery));
    assert!(retried.errors.is_empty(), "{retried:?}");
    assert_eq!(
        recovered.available_undo().unwrap().transaction_id,
        second.transaction_id
    );
    assert_ne!(
        recovered.available_undo().unwrap().transaction_id,
        first.transaction_id
    );
}

#[cfg(windows)]
struct ReadOnlyOnTruncateApplier {
    manifest: PathBuf,
}

#[cfg(windows)]
impl ConversationApplier for ReadOnlyOnTruncateApplier {
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
        reasonix_core::fileutil::set_file_mode(&self.manifest, 0o400).unwrap();
        Ok(())
    }

    fn restore_checkpoints(&self, _backup: &[u8]) -> Result<(), ConversationError> {
        Ok(())
    }
}

#[test]
#[cfg(windows)]
fn startup_undo_terminal_persistence_failure_stays_committing() {
    let (fixture, undo, manifest) = crash_conversation_undo();
    let recovered = restart(&fixture);
    let applier = ReadOnlyOnTruncateApplier {
        manifest: manifest.clone(),
    };

    let failed = recovered.recover_transactions_with_applier(Some(&applier));

    assert!(!failed.errors.is_empty());
    assert!(failed.notes.iter().any(|note| {
        note.phase == RecoveryPhase::PersistAborted
            && note.path.ends_with(format!("{}.json", undo.id))
    }));
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
    let (durable, _) = manifests(&fixture.dir)
        .into_iter()
        .find(|(transaction, _)| transaction.id == undo.id)
        .unwrap();
    assert_eq!(durable.state, TransactionState::Committing);
    assert_recovery_blocks_mutation(&recovered);

    reasonix_core::fileutil::set_file_mode(&manifest, 0o600).unwrap();
    let retried = recovered.recover_transactions_with_applier(Some(&MockApplier::default()));
    assert!(retried.errors.is_empty(), "{retried:?}");
    let (durable, _) = manifests(&fixture.dir)
        .into_iter()
        .find(|(transaction, _)| transaction.id == undo.id)
        .unwrap();
    assert_eq!(durable.state, TransactionState::Aborted);
    assert_recovery_allows_mutation(&recovered);
}
