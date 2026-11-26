use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::transaction_test_support::*;
use super::*;

struct PendingProbeApplier<'a> {
    store: &'a Store,
    pending_observations: AtomicUsize,
}

impl PendingProbeApplier<'_> {
    fn probe(&self) {
        if matches!(
            self.store.invalidate_undo(),
            Err(StoreError::UndoRecoveryPending)
        ) {
            self.pending_observations.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl ConversationApplier for PendingProbeApplier<'_> {
    fn apply_conversation_truncate(
        &self,
        _boundary: i64,
        _forward: &[u8],
    ) -> Result<(), ConversationError> {
        self.probe();
        Ok(())
    }

    fn restore_conversation(&self, _forward: &[u8]) -> Result<(), ConversationError> {
        self.probe();
        Ok(())
    }

    fn truncate_checkpoints(&self, _from_turn: i64) -> Result<(), ConversationError> {
        self.probe();
        Ok(())
    }

    fn restore_checkpoints(&self, _backup: &[u8]) -> Result<(), ConversationError> {
        self.probe();
        Ok(())
    }
}

fn conversation_plan(fixture: &Fixture, scope: RewindScope) -> RewindPlan {
    let plan = fixture.store.prepare_rewind(0, scope, 7, 5, true);
    assert!(plan.can_conversation);
    plan
}

#[test]
fn live_commit_tracks_pending_recovery_until_terminal_state() {
    let fixture = modify_add_delete_fixture();
    let plan = conversation_plan(&fixture, RewindScope::Both);
    let applier = PendingProbeApplier {
        store: &fixture.store,
        pending_observations: AtomicUsize::new(0),
    };

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

    assert_eq!(applier.pending_observations.load(Ordering::SeqCst), 2);
    fixture.store.require_undo_recovery_complete().unwrap();
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Committed
    );
}

#[test]
fn conversation_applier_truncates_then_truncates_checkpoints() {
    let fixture = modify_add_delete_fixture();
    let plan = conversation_plan(&fixture, RewindScope::Both);
    assert!(plan.can_files);
    let applier = MockApplier::default();
    let result = fixture
        .store
        .commit_rewind(
            &plan.plan_id,
            7,
            b"conversation-forward",
            Some(&applier),
            None,
        )
        .unwrap();

    assert!(result.ok);
    assert!(!result.undo_available);
    assert!(result.conversation_ok);
    assert_eq!(
        *applier.calls.lock().unwrap(),
        vec![
            ConversationCall::ApplyConversationTruncate {
                boundary: 5,
                forward: b"conversation-forward".to_vec(),
            },
            ConversationCall::TruncateCheckpoints(0),
        ]
    );
    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.state, TransactionState::Committed);
    assert_eq!(durable.conversation_forward, b"conversation-forward");
    assert!(!durable.checkpoint_backup.is_empty());
}

#[test]
fn later_conversation_failure_restores_conversation_and_checkpoint_backup() {
    let fixture = modify_add_delete_fixture();
    let plan = conversation_plan(&fixture, RewindScope::Both);
    let applier = MockApplier::default();
    let injection = TransactionInjection::fail(TransactionPhase::SyncRestoredOwnership);
    let error = fixture
        .store
        .commit_rewind(
            &plan.plan_id,
            7,
            b"conversation-forward",
            Some(&applier),
            Some(&injection),
        )
        .unwrap_err();
    assert!(
        error
            .phases()
            .contains(&TransactionPhase::SyncRestoredOwnership)
    );

    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.state, TransactionState::Aborted);
    let calls = applier.calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    assert!(matches!(
        calls[0],
        ConversationCall::ApplyConversationTruncate { .. }
    ));
    assert!(matches!(calls[1], ConversationCall::TruncateCheckpoints(0)));
    assert_eq!(
        calls[2],
        ConversationCall::RestoreConversation(b"conversation-forward".to_vec())
    );
    assert_eq!(
        calls[3],
        ConversationCall::RestoreCheckpoints(durable.checkpoint_backup.clone())
    );
    drop(calls);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
}

#[test]
fn conversation_restore_failure_keeps_manifest_committing_with_attribution() {
    let fixture = modify_add_delete_fixture();
    let plan = conversation_plan(&fixture, RewindScope::Both);
    let applier = MockApplier::default();
    applier.fail_at(ConversationPhase::TruncateCheckpoints);
    applier.fail_at(ConversationPhase::RestoreConversation);
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

    assert_eq!(
        error.conversation_phases(),
        vec![
            ConversationPhase::TruncateCheckpoints,
            ConversationPhase::TruncateCheckpoints,
            ConversationPhase::RestoreConversation,
            ConversationPhase::RestoreConversation,
        ]
    );
    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.state, TransactionState::Committing);
    assert!(!durable.error.is_empty());
    let calls = applier.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|call| matches!(call, ConversationCall::RestoreConversation(_)))
    );
    assert!(
        calls
            .iter()
            .any(|call| matches!(call, ConversationCall::RestoreCheckpoints(_)))
    );
}

fn assert_checkpoints_truncated(fixture: &Fixture) {
    assert!(fixture.store.list().is_empty());
    for turn in 0..=2 {
        assert!(!turn_path(&fixture.dir, turn, false).exists());
        assert!(!turn_path(&fixture.dir, turn, true).exists());
    }
    assert!(
        Store::new(Some(&fixture.dir), fixture.root.path())
            .list()
            .is_empty()
    );
}

#[test]
fn conversation_commit_without_applier_truncates_store_checkpoints() {
    let fixture = modify_add_delete_fixture();
    let plan = conversation_plan(&fixture, RewindScope::Conversation);
    let result = fixture
        .store
        .commit_rewind(&plan.plan_id, 7, b"forward", None, None)
        .unwrap();

    assert!(result.ok);
    assert!(!result.undo_available);
    assert_checkpoints_truncated(&fixture);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.state, TransactionState::Committed);
    assert!(durable.checkpoint_backup.is_empty());
}

#[test]
fn both_commit_without_applier_truncates_store_checkpoints() {
    let fixture = modify_add_delete_fixture();
    let plan = conversation_plan(&fixture, RewindScope::Both);
    assert!(plan.can_files);
    let result = fixture
        .store
        .commit_rewind(&plan.plan_id, 7, b"forward", None, None)
        .unwrap();

    assert!(result.ok);
    assert!(!result.undo_available);
    assert_checkpoints_truncated(&fixture);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
    assert_eq!(read(fixture.root.path(), "c.txt"), b"c-before");
    assert!(!fixture.root.path().join("b.txt").exists());
}

#[test]
fn conversation_compensation_without_applier_leaves_checkpoints_truncated() {
    let mut fixture = fixture();
    write(fixture.root.path(), "a.txt", "a-after");
    write(fixture.root.path(), "b.txt", "b-created");
    let checkpoints = [checkpoint(
        0,
        vec![
            owned_file(
                "a.txt",
                Some("a-before"),
                Some(true),
                &digest(b"a-after"),
                live_mode(),
            ),
            owned_file(
                "b.txt",
                None,
                Some(true),
                &digest(b"b-created"),
                live_mode(),
            ),
            owned_file("c.txt", Some("c-before"), Some(false), "", 0),
        ],
    )];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);
    let mut expired = checkpoint(1, Vec::new());
    expired.expired_file_payload = true;
    expired
        .coverage_gaps
        .push(CoverageGap::new(GAP_EXPIRED_PAYLOAD, EXPIRED_DETAIL));
    expired.recompute_coverage();
    write_json_atomic(&turn_path(&fixture.dir, 1, true), &expired).unwrap();
    fixture.store = Store::new(Some(&fixture.dir), fixture.root.path());
    fixture.store.begin(2, "future", 9).unwrap();
    let plan = conversation_plan(&fixture, RewindScope::Conversation);
    let injection = TransactionInjection::fail(TransactionPhase::SyncRestoredOwnership);

    let error = fixture
        .store
        .commit_rewind(&plan.plan_id, 7, b"forward", None, Some(&injection))
        .unwrap_err();

    assert!(
        error
            .phases()
            .contains(&TransactionPhase::SyncRestoredOwnership)
    );
    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.state, TransactionState::Aborted);
    assert!(durable.checkpoint_backup.is_empty());
    assert_checkpoints_truncated(&fixture);
    assert!(fixture.store.state().current.is_none());
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
}

#[test]
fn conversation_external_error_preserves_phase_and_source() {
    let dependency = std::io::Error::new(std::io::ErrorKind::NotConnected, "dependency offline");
    let error = ConversationError::external(ConversationPhase::RestoreCheckpoints, dependency);

    assert_eq!(error.phase(), ConversationPhase::RestoreCheckpoints);
    assert_eq!(error.phases(), vec![ConversationPhase::RestoreCheckpoints]);
    let source = std::error::Error::source(&error).expect("external source");
    assert_eq!(
        source
            .downcast_ref::<std::io::Error>()
            .expect("dependency error")
            .kind(),
        std::io::ErrorKind::NotConnected
    );

    let nested = ConversationError::Operation {
        phase: ConversationPhase::ApplyConversationTruncate,
        source: Box::new(error),
    };
    assert_eq!(
        nested.phases(),
        vec![
            ConversationPhase::ApplyConversationTruncate,
            ConversationPhase::RestoreCheckpoints,
        ]
    );
}

#[test]
fn checkpoint_backup_contains_future_checkpoints() {
    let fixture = modify_add_delete_fixture();
    let plan = conversation_plan(&fixture, RewindScope::Both);
    let applier = MockApplier::default();
    let injection = TransactionInjection::crash(TransactionPhase::CrashAfterPrepared);
    assert!(
        fixture
            .store
            .commit_rewind(
                &plan.plan_id,
                7,
                b"conversation-forward",
                Some(&applier),
                Some(&injection),
            )
            .is_err()
    );

    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.state, TransactionState::Prepared);
    assert_eq!(durable.truncate_from, 0);
    let backup: Vec<Checkpoint> = serde_json::from_slice(&durable.checkpoint_backup).unwrap();
    assert!(backup.iter().any(|checkpoint| checkpoint.turn == 0));
    assert!(fs::metadata(&durable.targets[0].publish_tmp).is_ok());
}

#[test]
fn checkpoint_backup_uses_compact_go_json_bytes() {
    let mut fixture = fixture();
    let sensitive = "<>&\u{2028}\u{2029}";
    let escaped = "\\u003c\\u003e\\u0026\\u2028\\u2029";
    let checkpoint = Checkpoint {
        schema_version: SCHEMA_V2,
        turn: 0,
        time: serde_json::from_value::<Checkpoint>(serde_json::json!({
            "turn": 0,
            "time": "2026-02-03T04:05:06.123456789Z"
        }))
        .unwrap()
        .time,
        prompt: format!("prompt {sensitive}"),
        msg_index: 9,
        session_id: "source".to_owned(),
        files: vec![FileSnap {
            path: format!("dir/{sensitive}/file.txt"),
            content: Some(format!("content {sensitive}")),
            mode: 0o600,
            ..FileSnap::default()
        }],
        coverage: Coverage::Complete,
        ..Checkpoint::default()
    };
    fixture.store.persist_checkpoint(&checkpoint).unwrap();
    fixture.store = Store::new(Some(&fixture.dir), fixture.root.path());
    let plan = conversation_plan(&fixture, RewindScope::Conversation);
    let applier = MockApplier::default();
    let injection = TransactionInjection::crash(TransactionPhase::CrashAfterPrepared);
    assert!(
        fixture
            .store
            .commit_rewind(
                &plan.plan_id,
                7,
                b"conversation-forward",
                Some(&applier),
                Some(&injection),
            )
            .is_err()
    );

    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.state, TransactionState::Prepared);
    let bytes = durable.checkpoint_backup;
    let expected = format!(
        r#"[{{"schemaVersion":2,"turn":0,"time":"2026-02-03T04:05:06.123456789Z","prompt":"prompt {escaped}","msgIndex":9,"sessionId":"source","files":[{{"path":"dir/{escaped}/file.txt","content":"content {escaped}","mode":384}}],"coverage":"complete"}}]"#
    );
    assert_eq!(bytes.as_slice(), expected.as_bytes());
    assert!(!bytes.contains(&b'\n'));
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(
        !text
            .chars()
            .any(|character| matches!(character, '<' | '>' | '&' | '\u{2028}' | '\u{2029}'))
    );
    let decoded: Vec<Checkpoint> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, vec![checkpoint]);
}
