use std::fs;

use chrono::Duration;

use crate::sessionstore::eventlog::record::CreatedAt;

use super::transaction_test_support::*;
use super::*;

fn commit(
    store: &Store,
    plan_id: &str,
    resolution: ConflictResolution,
) -> Result<RewindResult, FileRevertError> {
    store.commit_file_revert(plan_id, resolution)
}

fn rejected(error: FileRevertError) -> (FileRevertRejection, RewindResult) {
    match error {
        FileRevertError::Rejected { reason, result } => (reason, *result),
        other => panic!("expected rejected file revert, got {other:?}"),
    }
}

fn single_file(before: Option<&str>, after_existed: bool, after: &[u8]) -> Fixture {
    let mut fixture = fixture();
    if after_existed {
        write_bytes(fixture.root.path(), "a.txt", after);
    }
    let after_sha = if after_existed {
        digest(after)
    } else {
        String::new()
    };
    let checkpoints = [checkpoint(
        0,
        vec![owned_file(
            "a.txt",
            before,
            Some(after_existed),
            &after_sha,
            if after_existed { live_mode() } else { 0 },
        )],
    )];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);
    fixture
}

#[test]
fn file_revert_clean_keep_current_commits_one_code_target() {
    let fixture = single_file(Some("before"), true, b"after");
    write(fixture.root.path(), "unrelated.txt", "untouched");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);

    assert!(plan.can_files, "{plan:?}");
    assert!(plan.conflicts.is_empty());
    assert_eq!(plan.scope, RewindScope::Code);
    assert_eq!(plan.files, vec!["a.txt".to_owned()]);
    assert_eq!(plan.file_count, 1);
    assert_eq!(plan.session_revision, 7);
    assert!(!plan.can_conversation);

    let result = commit(
        &fixture.store,
        &plan.plan_id,
        ConflictResolution::KeepCurrent,
    )
    .unwrap();

    assert!(result.ok);
    assert!(result.undo_available);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"before");
    assert_eq!(read(fixture.root.path(), "unrelated.txt"), b"untouched");
    assert_eq!(result.files.len(), 1);
    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.schema_version, SCHEMA_V2);
    assert_eq!(durable.kind, "file_revert");
    assert_eq!(durable.scope, RewindScope::Code);
    assert_eq!(durable.targets.len(), 1);
    assert_eq!(durable.targets[0].path, "a.txt");
    assert!(durable.checkpoint_backup.is_empty());
    assert!(durable.conversation_forward.is_empty());
    assert!(!durable.has_boundary);
    assert!(matches!(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        ),
        Err(FileRevertError::UnknownOrExpiredPlan)
    ));
}

#[test]
fn file_revert_conflict_keep_current_is_successful_noop() {
    let fixture = single_file(Some("before"), true, b"after");
    write(fixture.root.path(), "a.txt", "manual-edit");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.can_files);
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].reason, CONFLICT_MANUAL_EDIT);

    let result = commit(
        &fixture.store,
        &plan.plan_id,
        ConflictResolution::KeepCurrent,
    )
    .unwrap();

    assert!(result.ok);
    assert!(!result.undo_available);
    assert!(result.transaction_id.is_empty());
    assert_eq!(read(fixture.root.path(), "a.txt"), b"manual-edit");
    assert_no_manifest(&fixture.dir);
    assert!(matches!(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        ),
        Err(FileRevertError::UnknownOrExpiredPlan)
    ));
}

#[test]
fn file_revert_overwrite_resolves_preview_conflict() {
    let fixture = single_file(Some("before"), true, b"after");
    write(fixture.root.path(), "a.txt", "manual-edit");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.can_files);
    assert_eq!(plan.conflicts[0].reason, CONFLICT_MANUAL_EDIT);

    let result = commit(
        &fixture.store,
        &plan.plan_id,
        ConflictResolution::OverwriteCheckpoint,
    )
    .unwrap();

    assert!(result.ok);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"before");
    assert_eq!(single_manifest(&fixture.dir).0.kind, "file_revert");
}

#[test]
fn file_revert_rejects_stale_preview_even_with_old_overwrite_approval() {
    let fixture = single_file(Some("before"), true, b"after");
    write(fixture.root.path(), "a.txt", "preview-edit");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.can_files);
    assert_eq!(plan.conflicts[0].reason, CONFLICT_MANUAL_EDIT);
    write(fixture.root.path(), "a.txt", "later-edit");

    let (reason, result) = rejected(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap_err(),
    );

    assert!(matches!(reason, FileRevertRejection::PreviewChanged { .. }));
    assert!(!result.ok);
    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(result.conflicts[0].reason, CONFLICT_STALE_PLAN);
    assert_eq!(result.conflicts[0].current_sha, digest(b"later-edit"));
    assert_eq!(read(fixture.root.path(), "a.txt"), b"later-edit");
    assert_no_manifest(&fixture.dir);
    assert!(matches!(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        ),
        Err(FileRevertError::UnknownOrExpiredPlan)
    ));
}

#[test]
fn file_revert_refuses_legacy_checkpoint_without_ownership_fingerprint() {
    let mut fixture = single_file(Some("before"), true, b"after");
    let mut legacy = checkpoint(0, vec![owned_file("a.txt", Some("before"), None, "", 0)]);
    legacy.schema_version = SCHEMA_V1;
    fixture.store = load(fixture.root.path(), &fixture.dir, &[legacy]);

    let plan = fixture.store.prepare_file_revert("a.txt", 7);

    assert!(plan.plan_id.is_empty());
    assert!(!plan.can_files);
    assert!(plan.legacy);
    assert_eq!(plan.coverage, Coverage::Legacy);
    assert_eq!(plan.disabled_reason, DISABLED_LEGACY);
    assert_eq!(plan.conflicts[0].reason, CONFLICT_COVERAGE_LEGACY);
    for resolution in [
        ConflictResolution::KeepCurrent,
        ConflictResolution::OverwriteCheckpoint,
    ] {
        assert!(matches!(
            commit(&fixture.store, &plan.plan_id, resolution),
            Err(FileRevertError::UnknownOrExpiredPlan)
        ));
    }
    assert_eq!(read(fixture.root.path(), "a.txt"), b"after");
    assert_no_manifest(&fixture.dir);
}

#[test]
fn file_revert_refuses_incomplete_present_ownership_fingerprint() {
    let mut fixture = single_file(Some("before"), true, b"after");
    let incomplete = checkpoint(
        0,
        vec![owned_file(
            "a.txt",
            Some("before"),
            Some(true),
            "",
            live_mode(),
        )],
    );
    fixture.store = load(fixture.root.path(), &fixture.dir, &[incomplete]);

    let plan = fixture.store.prepare_file_revert("a.txt", 7);

    assert!(plan.plan_id.is_empty());
    assert!(!plan.can_files);
    assert!(plan.legacy);
    assert_eq!(plan.coverage, Coverage::Legacy);
    assert_eq!(plan.disabled_reason, DISABLED_LEGACY);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"after");
}

#[test]
fn file_revert_requires_latest_ownership_fingerprint() {
    let mut fixture = single_file(Some("first"), true, b"after");
    let checkpoints = [
        checkpoint(
            0,
            vec![owned_file(
                "a.txt",
                Some("first"),
                Some(true),
                &digest(b"intermediate"),
                live_mode(),
            )],
        ),
        checkpoint(1, vec![owned_file("a.txt", Some("second"), None, "", 0)]),
    ];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);

    let plan = fixture.store.prepare_file_revert("a.txt", 7);

    assert!(plan.plan_id.is_empty());
    assert!(!plan.can_files);
    assert!(plan.legacy);
    assert_eq!(plan.coverage, Coverage::Legacy);
    assert_eq!(plan.disabled_reason, DISABLED_LEGACY);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"after");
}

#[test]
fn file_revert_uses_earliest_preimage_and_latest_valid_ownership() {
    let mut fixture = single_file(Some("first"), true, b"after");
    let checkpoints = [
        checkpoint(
            0,
            vec![owned_file(
                "a.txt",
                Some("first"),
                Some(true),
                &digest(b"intermediate"),
                live_mode(),
            )],
        ),
        checkpoint(
            1,
            vec![owned_file(
                "a.txt",
                Some("second"),
                Some(true),
                &digest(b"after"),
                live_mode(),
            )],
        ),
    ];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.can_files, "{plan:?}");
    assert!(plan.conflicts.is_empty());

    assert!(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap()
        .ok
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"first");
}

#[test]
fn file_revert_consumes_clean_plan_before_stale_verdict() {
    let fixture = single_file(Some("before"), true, b"after");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.conflicts.is_empty());
    write(fixture.root.path(), "a.txt", "external-edit");

    let (reason, result) = rejected(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::KeepCurrent,
        )
        .unwrap_err(),
    );

    assert!(matches!(reason, FileRevertRejection::PreviewChanged { .. }));
    assert!(!result.ok);
    assert_eq!(result.conflicts[0].reason, CONFLICT_STALE_PLAN);
    assert!(matches!(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        ),
        Err(FileRevertError::UnknownOrExpiredPlan)
    ));
}

#[test]
fn file_revert_consumes_disabled_plan_before_verdict() {
    let mut fixture = single_file(Some("before"), true, b"after");
    let missing = checkpoint(
        0,
        vec![FileSnap {
            sha256: "0".repeat(64),
            after_existed: Some(true),
            after_sha256: digest(b"after"),
            after_mode: live_mode(),
            ..owned_file("a.txt", None, Some(true), &digest(b"after"), live_mode())
        }],
    );
    fixture.store = load(fixture.root.path(), &fixture.dir, &[missing]);
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(!plan.plan_id.is_empty());
    assert!(!plan.can_files);
    assert_eq!(plan.conflicts[0].reason, CONFLICT_MISSING_PAYLOAD);

    let (reason, result) = rejected(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap_err(),
    );

    assert!(matches!(
        reason,
        FileRevertRejection::Disabled(FileRevertDisabledReason::MissingPayload)
    ));
    assert!(!result.ok);
    assert_eq!(result.conflicts[0].reason, CONFLICT_MISSING_PAYLOAD);
    assert!(matches!(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::KeepCurrent,
        ),
        Err(FileRevertError::UnknownOrExpiredPlan)
    ));
    assert_no_manifest(&fixture.dir);
}

#[test]
fn file_revert_rejects_unknown_and_empty_plan_ids() {
    let fixture = single_file(Some("before"), true, b"after");
    for plan_id in ["", "missing-plan"] {
        assert!(matches!(
            commit(
                &fixture.store,
                plan_id,
                ConflictResolution::OverwriteCheckpoint,
            ),
            Err(FileRevertError::UnknownOrExpiredPlan)
        ));
    }
    assert_no_manifest(&fixture.dir);
}

#[test]
fn file_revert_take_ignores_ten_minute_expiry() {
    let fixture = single_file(Some("before"), true, b"after");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    {
        let mut registry = fixture.store.runtime.plans.lock().unwrap();
        let prepared = registry.files.entries.get_mut(&plan.plan_id).unwrap();
        prepared.prepared_at = CreatedAt::from_datetime(
            CreatedAt::now().get() - Duration::minutes(10) - Duration::seconds(1),
        );
    }

    let result = commit(
        &fixture.store,
        &plan.plan_id,
        ConflictResolution::OverwriteCheckpoint,
    )
    .unwrap();

    assert!(result.ok);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"before");
}

#[test]
fn file_revert_new_prepare_opportunistically_evicts_stale_plan() {
    let fixture = single_file(Some("before"), true, b"after");
    let stale = fixture.store.prepare_file_revert("a.txt", 7);
    {
        let mut registry = fixture.store.runtime.plans.lock().unwrap();
        registry
            .files
            .entries
            .get_mut(&stale.plan_id)
            .unwrap()
            .prepared_at = CreatedAt::from_datetime(
            CreatedAt::now().get() - Duration::minutes(10) - Duration::seconds(1),
        );
    }
    let fresh = fixture.store.prepare_file_revert("a.txt", 7);

    assert!(matches!(
        commit(
            &fixture.store,
            &stale.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        ),
        Err(FileRevertError::UnknownOrExpiredPlan)
    ));
    assert!(
        commit(
            &fixture.store,
            &fresh.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap()
        .ok
    );
}

#[test]
fn file_revert_rechecks_workspace_generation_under_barrier() {
    let fixture = single_file(Some("before"), true, b"after");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    let writer = fixture.store.barrier().try_enter_write().unwrap();
    drop(writer);

    let (reason, result) = rejected(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap_err(),
    );

    assert!(matches!(
        reason,
        FileRevertRejection::WorkspaceChanged { .. }
    ));
    assert!(!result.ok);
    assert_eq!(result.conflicts[0].reason, CONFLICT_STALE_PLAN);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"after");
    assert_no_manifest(&fixture.dir);
}

#[test]
fn file_revert_rechecks_active_writers_under_barrier() {
    let fixture = single_file(Some("before"), true, b"after");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    fixture.store.set_active_writers(vec![active_writer()]);

    let (reason, result) = rejected(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap_err(),
    );

    assert!(matches!(reason, FileRevertRejection::ActiveWriters));
    assert!(!result.ok);
    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(result.conflicts[0].path, "a.txt");
    assert_eq!(result.conflicts[0].reason, CONFLICT_BUSY_WRITER);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"after");
    assert_no_manifest(&fixture.dir);
}

#[test]
fn file_revert_busy_barrier_has_typed_writer_conflict() {
    let fixture = single_file(Some("before"), true, b"after");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    let exclusive = fixture.store.barrier().try_enter_exclusive().unwrap();

    let (reason, result) = rejected(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap_err(),
    );

    assert!(matches!(reason, FileRevertRejection::BarrierBusy));
    assert!(!result.ok);
    assert_eq!(result.conflicts[0].reason, CONFLICT_BUSY_WRITER);
    assert!(matches!(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        ),
        Err(FileRevertError::UnknownOrExpiredPlan)
    ));
    drop(exclusive);
    assert_no_manifest(&fixture.dir);
}

#[test]
fn file_revert_waits_for_pending_undo_recovery() {
    let (fixture, original) = committed_fixture();
    let injection =
        TransactionInjection::crash(TransactionPhase::CrashAfterConversationBeforeFinalize);
    assert!(
        fixture
            .store
            .undo_rewind(&original.transaction_id, None, Some(&injection))
            .is_err()
    );
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.can_files, "{plan:?}");
    let manifests_before = transaction_paths(&fixture.dir);

    let (reason, result) = rejected(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap_err(),
    );

    assert!(matches!(reason, FileRevertRejection::UndoRecoveryPending));
    assert!(!result.ok);
    assert_eq!(result.conflicts[0].reason, CONFLICT_BOUNDARY_INVALID);
    assert_eq!(transaction_paths(&fixture.dir), manifests_before);
}

#[test]
fn file_revert_absent_to_present_restores_delete_and_undo() {
    let fixture = single_file(None, true, b"created");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.can_files, "{plan:?}");

    let result = commit(
        &fixture.store,
        &plan.plan_id,
        ConflictResolution::OverwriteCheckpoint,
    )
    .unwrap();

    assert!(result.ok);
    assert!(!fixture.root.path().join("a.txt").exists());
    let durable = single_manifest(&fixture.dir).0;
    assert!(!durable.targets[0].restore_existed);
    assert!(durable.targets[0].forward_existed);
    assert_eq!(durable.targets[0].action, "delete");

    assert!(
        fixture
            .store
            .undo_rewind(&result.transaction_id, None, None)
            .unwrap()
            .ok
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"created");
}

#[test]
fn file_revert_present_to_absent_restores_file_and_undo_deletes_it() {
    let fixture = single_file(Some("before"), false, b"");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.can_files, "{plan:?}");

    let result = commit(
        &fixture.store,
        &plan.plan_id,
        ConflictResolution::OverwriteCheckpoint,
    )
    .unwrap();

    assert!(result.ok);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"before");
    let durable = single_manifest(&fixture.dir).0;
    assert!(durable.targets[0].restore_existed);
    assert!(!durable.targets[0].forward_existed);

    assert!(
        fixture
            .store
            .undo_rewind(&result.transaction_id, None, None)
            .unwrap()
            .ok
    );
    assert!(!fixture.root.path().join("a.txt").exists());
}

#[test]
fn file_revert_present_empty_forward_payload_survives_undo() {
    let fixture = single_file(Some("before"), true, b"");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.can_files, "{plan:?}");

    let result = commit(
        &fixture.store,
        &plan.plan_id,
        ConflictResolution::OverwriteCheckpoint,
    )
    .unwrap();

    assert_eq!(read(fixture.root.path(), "a.txt"), b"before");
    let durable = single_manifest(&fixture.dir).0;
    let target = &durable.targets[0];
    assert!(target.forward_existed);
    assert_eq!(target.forward_sha, digest(b""));
    assert_eq!(target.forward_blob, digest(b""));
    assert!(target.restore_existed);

    assert!(
        fixture
            .store
            .undo_rewind(&result.transaction_id, None, None)
            .unwrap()
            .ok
    );
    assert!(fixture.root.path().join("a.txt").is_file());
    assert!(read(fixture.root.path(), "a.txt").is_empty());
}

#[test]
fn file_revert_blob_preimage_takes_precedence_over_inline_text() {
    let mut fixture = fixture();
    let restore = b"\xff\xferaw\x00payload";
    let blob = fixture.store.blobs().unwrap().put(restore).unwrap();
    write(fixture.root.path(), "a.txt", "after");
    let checkpoints = [checkpoint(
        0,
        vec![FileSnap {
            content: Some("wrong-inline-text".to_owned()),
            blob_ref: blob.clone(),
            sha256: blob,
            ..owned_file("a.txt", None, Some(true), &digest(b"after"), live_mode())
        }],
    )];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.can_files, "{plan:?}");

    assert!(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap()
        .ok
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), restore);
}

#[test]
fn file_revert_falls_back_to_inline_text_when_blob_is_missing() {
    let mut fixture = fixture();
    write(fixture.root.path(), "a.txt", "after");
    let checkpoints = [checkpoint(
        0,
        vec![FileSnap {
            content: Some("inline-before".to_owned()),
            blob_ref: "0".repeat(64),
            sha256: digest(b"inline-before"),
            ..owned_file("a.txt", None, Some(true), &digest(b"after"), live_mode())
        }],
    )];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.can_files, "{plan:?}");

    let result = commit(
        &fixture.store,
        &plan.plan_id,
        ConflictResolution::OverwriteCheckpoint,
    )
    .unwrap();

    assert!(result.ok);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"inline-before");
    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.targets[0].restore_blob, digest(b"inline-before"));
}

#[test]
fn file_revert_restores_legacy_sha_addressed_blob() {
    let mut fixture = fixture();
    let restore = b"legacy-sha-payload";
    let blob = fixture.store.blobs().unwrap().put(restore).unwrap();
    write(fixture.root.path(), "a.txt", "after");
    let checkpoints = [checkpoint(
        0,
        vec![FileSnap {
            sha256: blob.clone(),
            ..owned_file("a.txt", None, Some(true), &digest(b"after"), live_mode())
        }],
    )];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.can_files, "{plan:?}");

    assert!(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap()
        .ok
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), restore);
}

#[test]
fn file_revert_inline_fallback_uses_current_encoding() {
    let mut fixture = fixture();
    let before =
        reasonix_core::fileencoding::encode("before €", reasonix_core::fileencoding::Kind::Gb18030)
            .unwrap();
    let after =
        reasonix_core::fileencoding::encode("after €", reasonix_core::fileencoding::Kind::Gb18030)
            .unwrap();
    write_bytes(fixture.root.path(), "a.txt", &after);
    let checkpoints = [checkpoint(
        0,
        vec![FileSnap {
            content: Some("before €".to_owned()),
            sha256: digest(&before),
            ..owned_file("a.txt", None, Some(true), &digest(&after), live_mode())
        }],
    )];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.can_files, "{plan:?}");

    assert!(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap()
        .ok
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), before);
}

#[test]
fn file_revert_resolves_cleaned_absolute_path_and_preserves_recorded_spelling() {
    let mut fixture = fixture();
    write(fixture.root.path(), "a.txt", "after");
    let checkpoints = [checkpoint(
        0,
        vec![owned_file(
            "nested/../a.txt",
            Some("before"),
            Some(true),
            &digest(b"after"),
            live_mode(),
        )],
    )];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);
    let caller = fixture.root.path().join("a.txt");
    let plan = fixture.store.prepare_file_revert(&caller, 7);

    assert!(plan.can_files, "{plan:?}");
    assert_eq!(plan.path, "nested/../a.txt");
    assert_eq!(plan.file_count, 1);
    assert!(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap()
        .ok
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"before");
    assert_eq!(
        single_manifest(&fixture.dir).0.targets[0].path,
        "nested/../a.txt"
    );
}

#[test]
fn file_revert_unsafe_path_is_disabled_with_typed_conflict() {
    let mut fixture = fixture();
    let outside = fixture._temp.path().join("outside.txt");
    fs::write(&outside, b"outside").unwrap();
    let checkpoints = [checkpoint(
        0,
        vec![owned_file(
            outside.to_string_lossy().as_ref(),
            Some("before"),
            Some(true),
            &digest(b"outside"),
            live_mode(),
        )],
    )];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);

    let plan = fixture.store.prepare_file_revert(&outside, 7);

    assert!(!plan.can_files);
    assert_eq!(plan.disabled_reason, "path unsafe");
    assert_eq!(plan.conflicts[0].reason, CONFLICT_PATH_UNSAFE);
    let (reason, result) = rejected(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap_err(),
    );
    assert!(matches!(
        reason,
        FileRevertRejection::Disabled(FileRevertDisabledReason::PathUnsafe)
    ));
    assert_eq!(result.conflicts[0].reason, CONFLICT_PATH_UNSAFE);
    assert_eq!(fs::read(&outside).unwrap(), b"outside");
    assert_no_manifest(&fixture.dir);
}

#[test]
fn file_revert_missing_payload_cannot_be_enabled_by_either_resolution() {
    for resolution in [
        ConflictResolution::KeepCurrent,
        ConflictResolution::OverwriteCheckpoint,
    ] {
        let mut fixture = single_file(Some("before"), true, b"after");
        let missing = checkpoint(
            0,
            vec![FileSnap {
                sha256: "0".repeat(64),
                after_existed: Some(true),
                after_sha256: digest(b"after"),
                after_mode: live_mode(),
                ..owned_file("a.txt", None, Some(true), &digest(b"after"), live_mode())
            }],
        );
        fixture.store = load(fixture.root.path(), &fixture.dir, &[missing]);
        let plan = fixture.store.prepare_file_revert("a.txt", 7);
        assert!(!plan.can_files);

        let (reason, result) =
            rejected(commit(&fixture.store, &plan.plan_id, resolution).unwrap_err());
        assert!(matches!(
            reason,
            FileRevertRejection::Disabled(FileRevertDisabledReason::MissingPayload)
        ));
        assert!(!result.ok);
        assert_eq!(result.conflicts[0].reason, CONFLICT_MISSING_PAYLOAD);
        assert_eq!(read(fixture.root.path(), "a.txt"), b"after");
        assert_no_manifest(&fixture.dir);
    }
}

#[test]
fn file_revert_prepare_propagates_current_capture_error_as_coverage_gap() {
    let mut fixture = single_file(Some("before"), true, b"after");
    fixture.store.config.max_file_bytes = 4;
    write(fixture.root.path(), "a.txt", "too-large");

    let plan = fixture.store.prepare_file_revert("a.txt", 7);

    assert!(plan.plan_id.is_empty());
    assert!(!plan.can_files);
    assert_eq!(plan.coverage, Coverage::Partial);
    assert!(
        plan.coverage_gaps
            .iter()
            .any(|gap| { gap.reason == GAP_OVERSIZED && gap.path == "a.txt" })
    );
    assert!(!plan.disabled_reason.is_empty());
    assert_no_manifest(&fixture.dir);
}

#[test]
fn file_revert_commit_propagates_preview_capture_error() {
    let mut fixture = single_file(Some("before"), true, b"after");
    fixture.store.config.max_file_bytes = 16;
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    assert!(plan.can_files, "{plan:?}");
    write(fixture.root.path(), "a.txt", "this-file-is-now-too-large");

    let (reason, result) = rejected(
        commit(
            &fixture.store,
            &plan.plan_id,
            ConflictResolution::OverwriteCheckpoint,
        )
        .unwrap_err(),
    );

    assert!(matches!(
        reason,
        FileRevertRejection::PreviewUnavailable(
            CaptureError::AtPath { source, .. }
        ) if matches!(source.as_ref(), CaptureError::Oversized { .. })
    ));
    assert!(!result.ok);
    assert_eq!(result.conflicts[0].reason, CONFLICT_STALE_PLAN);
    assert!(
        result
            .coverage_gaps
            .iter()
            .any(|gap| gap.reason == GAP_OVERSIZED)
    );
    assert_no_manifest(&fixture.dir);
}

#[test]
fn file_revert_target_propagates_forward_capture_error() {
    let fixture = single_file(Some("before"), true, b"after");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    let revision = fixture.store.earliest_revisions(0)["a.txt"].clone();
    fs::remove_file(fixture.root.path().join("a.txt")).unwrap();
    fs::create_dir(fixture.root.path().join("a.txt")).unwrap();

    let error = transaction_prepare::prepare_file_revert(&fixture.store, &plan, &revision, None)
        .unwrap_err();

    assert!(matches!(
        error,
        TransactionError::Operation { phase: TransactionPhase::Prepare, source, .. }
            if matches!(
                source.as_ref(),
                TransactionError::Capture(CaptureError::AtPath { source, .. })
                    if matches!(source.as_ref(), CaptureError::Directory { .. })
            )
    ));
    assert!(fixture.root.path().join("a.txt").is_dir());
    assert_no_manifest(&fixture.dir);
}

#[test]
fn file_revert_durable_commit_recovers_offer_and_public_undo() {
    let fixture = single_file(Some("before"), true, b"after");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    let result = commit(
        &fixture.store,
        &plan.plan_id,
        ConflictResolution::OverwriteCheckpoint,
    )
    .unwrap();
    assert!(result.ok);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"before");
    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.kind, "file_revert");
    assert_eq!(durable.scope, RewindScope::Code);
    assert_eq!(durable.targets.len(), 1);
    assert_eq!(durable.state, TransactionState::Committed);

    let recovered = Store::new(Some(&fixture.dir), fixture.root.path());

    assert_eq!(read(fixture.root.path(), "a.txt"), b"before");
    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.state, TransactionState::Committed);
    let offer = recovered.current_undo_offer().unwrap();
    assert_eq!(offer.id, durable.id);
    assert_eq!(offer.targets.len(), 1);
    assert!(recovered.undo_rewind(&offer.id, None, None).unwrap().ok);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"after");
}

#[test]
fn file_revert_prepared_crash_is_aborted_without_target_mutation() {
    let fixture = single_file(Some("before"), true, b"after");
    let plan = fixture.store.prepare_file_revert("a.txt", 7);
    let injection = TransactionInjection::crash(TransactionPhase::CrashAfterPrepared);
    assert!(
        fixture
            .store
            .commit_file_revert_with_injection(
                &plan.plan_id,
                ConflictResolution::OverwriteCheckpoint,
                Some(&injection),
            )
            .is_err()
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"after");
    let durable = single_manifest(&fixture.dir).0;
    assert_eq!(durable.kind, "file_revert");
    assert_eq!(durable.state, TransactionState::Prepared);
    assert!(!durable.targets[0].published);

    let recovered = Store::new(Some(&fixture.dir), fixture.root.path());

    assert_eq!(read(fixture.root.path(), "a.txt"), b"after");
    assert_eq!(
        single_manifest(&fixture.dir).0.state,
        TransactionState::Aborted
    );
    assert!(recovered.current_undo_offer().is_none());
    assert!(
        recovered
            .recovery_notes()
            .iter()
            .any(|note| { note.phase == RecoveryPhase::AbortPrepared })
    );
}
