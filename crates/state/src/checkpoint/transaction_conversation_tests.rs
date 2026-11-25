use std::fs;

use super::transaction_test_support::*;
use super::*;

fn conversation_plan(fixture: &Fixture, scope: RewindScope) -> RewindPlan {
    let plan = fixture.store.prepare_rewind(0, scope, 7, 5, true);
    assert!(plan.can_conversation);
    plan
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

#[test]
fn conversation_commit_requires_applier() {
    let fixture = modify_add_delete_fixture();
    let plan = conversation_plan(&fixture, RewindScope::Conversation);
    let error = fixture
        .store
        .commit_rewind(&plan.plan_id, 7, b"forward", None, None)
        .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::MissingConversationApplier
    ));
    assert_no_manifest(&fixture.dir);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
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
