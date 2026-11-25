use std::fs;
use std::path::{Path, PathBuf};

use chrono::Duration;
use tempfile::TempDir;

use crate::sessionstore::eventlog::record::CreatedAt;

use super::*;

fn fixture() -> (TempDir, TempDir, PathBuf, Store) {
    let temp = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = temp.path().join("session.ckpt");
    let store = Store::new(Some(&dir), root.path());
    (temp, root, dir, store)
}

fn write(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, content).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
    }
}

fn live_mode() -> u32 {
    #[cfg(windows)]
    {
        0o666
    }
    #[cfg(not(windows))]
    {
        0o644
    }
}

fn checkpoint(turn: i64, files: Vec<FileSnap>) -> Checkpoint {
    let coverage = if files.is_empty() {
        Coverage::None
    } else {
        Coverage::Complete
    };
    Checkpoint {
        schema_version: SCHEMA_V2,
        turn,
        time: CreatedAt::now(),
        coverage,
        files,
        ..Checkpoint::default()
    }
}

fn owned_file(
    path: &str,
    before: Option<&str>,
    after_existed: Option<bool>,
    after_sha: &str,
    after_mode: u32,
) -> FileSnap {
    FileSnap {
        path: path.to_owned(),
        content: before.map(str::to_owned),
        sha256: before
            .map(|content| digest(content.as_bytes()))
            .unwrap_or_default(),
        mode: 0o644,
        after_existed,
        after_sha256: after_sha.to_owned(),
        after_mode,
        ..FileSnap::default()
    }
}

fn load(root: &Path, dir: &Path, checkpoints: &[Checkpoint]) -> Store {
    fs::create_dir_all(dir).unwrap();
    for checkpoint in checkpoints {
        fs::write(
            turn_path(dir, checkpoint.turn, false),
            serde_json::to_vec(checkpoint).unwrap(),
        )
        .unwrap();
    }
    Store::new(Some(dir), root)
}

#[test]
fn rewind_plan_lookup_and_take_are_single_use() {
    let (_temp, root, _dir, store) = fixture();
    let plan = store.prepare_rewind(3, RewindScope::Conversation, 17, 4, true);

    assert_eq!(store.rewind_plan(&plan.plan_id).unwrap(), plan);
    assert_eq!(store.take_rewind_plan(&plan.plan_id).unwrap(), plan);
    assert_eq!(
        store.take_rewind_plan(&plan.plan_id),
        Err(RewindError::UnknownOrExpiredPlan)
    );
    assert_eq!(
        store.validate_plan_session_revision(&plan.plan_id, 17),
        Err(RewindError::UnknownOrExpiredPlan)
    );
    assert_eq!(plan.turn, 3);
    assert_eq!(plan.scope, RewindScope::Conversation);
    assert_eq!(plan.session_revision, 17);
    assert_eq!(plan.boundary_index, 4);
    assert!(plan.has_boundary);
    assert!(plan.can_conversation);
    assert!(!plan.created_at.is_zero());
    assert!(!plan.plan_id.is_empty());
    drop(root);
}

#[test]
fn rewind_plan_validation_rejects_changed_session_revision() {
    let (_temp, _root, _dir, store) = fixture();
    let plan = store.prepare_rewind(0, RewindScope::Conversation, 5, 0, true);

    assert_eq!(
        store.validate_plan_session_revision(&plan.plan_id, 6),
        Err(RewindError::SessionRevisionMismatch)
    );
    store
        .validate_plan_session_revision(&plan.plan_id, 5)
        .unwrap();
    assert_eq!(store.rewind_plan(&plan.plan_id).unwrap(), plan);
}

#[test]
fn rewind_plan_expiry_is_deterministic_and_indistinguishable() {
    let (_temp, _root, _dir, store) = fixture();
    let plan = store.prepare_rewind(0, RewindScope::Conversation, 9, 0, true);
    let prepared_at = plan.created_at.get() - Duration::minutes(1);
    {
        let mut registry = store.runtime.plans.lock().unwrap();
        registry.entries.get_mut(&plan.plan_id).unwrap().prepared_at =
            CreatedAt::from_datetime(prepared_at);
    }

    let at_ten_minutes = CreatedAt::from_datetime(prepared_at + Duration::minutes(10));
    store
        .validate_plan_session_revision_at(&plan.plan_id, 9, at_ten_minutes)
        .unwrap();
    let looked_up = store.rewind_plan(&plan.plan_id).unwrap();
    assert_eq!(looked_up.created_at, plan.created_at);

    let after_ten_minutes =
        CreatedAt::from_datetime(prepared_at + Duration::minutes(10) + Duration::nanoseconds(1));
    let expired = store
        .validate_plan_session_revision_at(&plan.plan_id, 9, after_ten_minutes)
        .unwrap_err();
    let unknown = store
        .validate_plan_session_revision("missing-plan", 9)
        .unwrap_err();
    assert_eq!(expired, RewindError::UnknownOrExpiredPlan);
    assert_eq!(unknown, expired);
    assert_eq!(unknown.to_string(), expired.to_string());

    let taken = store.prepare_rewind(1, RewindScope::Conversation, 9, 1, true);
    {
        let mut registry = store.runtime.plans.lock().unwrap();
        let prepared = registry.entries.get_mut(&taken.plan_id).unwrap();
        prepared.prepared_at = CreatedAt::from_datetime(
            CreatedAt::now().get() - Duration::minutes(10) - Duration::seconds(1),
        );
    }
    assert_eq!(
        store.take_rewind_plan(&taken.plan_id),
        Err(RewindError::UnknownOrExpiredPlan)
    );
}

#[test]
fn store_barrier_is_shared_with_rewind_plan_tokens() {
    let (_temp, root, _dir, store) = fixture();
    let barrier = store.barrier().clone();
    let first = store.prepare_rewind(0, RewindScope::Conversation, 1, 0, true);
    assert_eq!(first.workspace_token, barrier.generation().to_string());

    let permit = barrier.try_enter_write().unwrap();
    drop(permit);
    let second = store.prepare_rewind(1, RewindScope::Conversation, 1, 1, true);
    assert_eq!(second.workspace_token, barrier.generation().to_string());
    assert_ne!(first.workspace_token, second.workspace_token);
    drop(root);
}

#[test]
fn active_writers_disable_both_halves_with_one_conflict_each() {
    let (_temp, root, _dir, _store) = fixture();
    write(root.path(), "a.txt", "after");
    let checkpoints = [checkpoint(
        0,
        vec![owned_file(
            "a.txt",
            Some("before"),
            Some(true),
            &digest(b"after"),
            live_mode(),
        )],
    )];
    let store = load(
        root.path(),
        &_temp.path().join("writers.ckpt"),
        &checkpoints,
    );
    let writers = vec![
        ActiveWriter {
            id: "writer-a".into(),
            kind: "background".into(),
            ..ActiveWriter::default()
        },
        ActiveWriter {
            id: "writer-b".into(),
            kind: "background".into(),
            ..ActiveWriter::default()
        },
    ];
    store.set_active_writers(writers.clone());

    let plan = store.prepare_rewind(0, RewindScope::Both, 1, 0, true);
    assert_eq!(plan.active_writers, writers);
    assert!(!plan.can_files);
    assert!(!plan.can_conversation);
    assert_eq!(plan.disabled_reason, "active background writer");
    assert_eq!(plan.conflicts.len(), 2);
    assert!(
        plan.conflicts.iter().all(|conflict| {
            conflict.path.is_empty() && conflict.reason == CONFLICT_BUSY_WRITER
        })
    );
}

#[test]
fn combined_rewind_without_tracked_files_is_an_atomic_file_noop() {
    let (_temp, root, _dir, store) = fixture();
    let plan = store.prepare_rewind(4, RewindScope::Both, 2, 7, true);

    assert!(plan.files.is_empty());
    assert_eq!(plan.file_count, 0);
    assert_eq!(plan.coverage, Coverage::None);
    assert!(plan.can_files);
    assert!(plan.can_conversation);
    assert!(plan.conflicts.is_empty());
    assert!(plan.disabled_reason.is_empty());
    drop(root);
}

#[test]
fn conversation_rewind_requires_a_boundary() {
    let (_temp, root, _dir, store) = fixture();
    let plan = store.prepare_rewind(0, RewindScope::Conversation, 1, 0, false);

    assert!(!plan.can_files);
    assert!(!plan.can_conversation);
    assert_eq!(plan.disabled_reason, "conversation boundary unavailable");
    assert!(plan.conflicts.is_empty());
    drop(root);
}

#[test]
fn rewind_coverage_none_disables_code_without_captures() {
    let (_temp, root, _dir, store) = fixture();
    let plan = store.prepare_rewind(0, RewindScope::Code, 1, 0, false);

    assert_eq!(plan.coverage, Coverage::None);
    assert!(!plan.legacy);
    assert!(!plan.expired_file_payload);
    assert!(!plan.can_files);
    assert_eq!(plan.disabled_reason, "no file captures");
    drop(root);
}

#[test]
fn rewind_coverage_partial_aggregates_gaps_and_allows_clean_files() {
    let (_temp, root, _dir, _store) = fixture();
    write(root.path(), "a.txt", "after");
    let gap = CoverageGap::new(GAP_BASH_SIDE_EFFECT, "untracked shell write");
    let mut partial = checkpoint(
        0,
        vec![owned_file(
            "a.txt",
            Some("before"),
            Some(true),
            &digest(b"after"),
            live_mode(),
        )],
    );
    partial.coverage = Coverage::Partial;
    partial.coverage_gaps.push(gap.clone());
    let checkpoints = [partial, checkpoint(1, vec![])];
    let store = load(
        root.path(),
        &_temp.path().join("partial.ckpt"),
        &checkpoints,
    );

    let (coverage, gaps, legacy, expired) = store.coverage_from_turn(0);
    assert_eq!(coverage, Coverage::Partial);
    assert_eq!(gaps, vec![gap.clone()]);
    assert!(!legacy);
    assert!(!expired);
    let plan = store.prepare_rewind(0, RewindScope::Code, 1, 0, false);
    assert_eq!(plan.coverage, Coverage::Partial);
    assert_eq!(plan.coverage_gaps, vec![gap]);
    assert!(plan.can_files);
    assert!(plan.conflicts.is_empty());
}

#[test]
fn rewind_coverage_legacy_disables_unverified_restore() {
    let (_temp, root, _dir, _store) = fixture();
    let mut legacy = checkpoint(0, vec![owned_file("a.txt", Some("before"), None, "", 0)]);
    legacy.schema_version = SCHEMA_V1;
    let store = load(root.path(), &_temp.path().join("legacy.ckpt"), &[legacy]);

    let (coverage, gaps, is_legacy, expired) = store.coverage_from_turn(0);
    assert_eq!(coverage, Coverage::Legacy);
    assert!(is_legacy);
    assert!(!expired);
    assert!(gaps.iter().any(|gap| gap.reason == GAP_LEGACY_UNVERIFIED));
    let plan = store.prepare_rewind(0, RewindScope::Code, 1, 0, false);
    assert!(plan.legacy);
    assert!(!plan.can_files);
    assert_eq!(
        plan.disabled_reason,
        "legacy checkpoint cannot verify later manual edits"
    );
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].reason, CONFLICT_COVERAGE_LEGACY);
}

#[test]
fn rewind_coverage_expired_disables_restore_and_reports_gap() {
    let (_temp, root, _dir, _store) = fixture();
    let mut expired = checkpoint(
        0,
        vec![owned_file(
            "a.txt",
            Some("before"),
            Some(true),
            &digest(b"after"),
            live_mode(),
        )],
    );
    expired.expired_file_payload = true;
    expired.coverage = Coverage::Partial;
    let store = load(root.path(), &_temp.path().join("expired.ckpt"), &[expired]);

    let (coverage, gaps, legacy, is_expired) = store.coverage_from_turn(0);
    assert_eq!(coverage, Coverage::Partial);
    assert!(!legacy);
    assert!(is_expired);
    assert!(gaps.iter().any(|gap| gap.reason == GAP_EXPIRED_PAYLOAD));
    let plan = store.prepare_rewind(0, RewindScope::Code, 1, 0, false);
    assert!(plan.expired_file_payload);
    assert!(!plan.can_files);
    assert_eq!(plan.disabled_reason, "file recovery payload expired");
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].reason, CONFLICT_EXPIRED);
}

#[test]
fn rewind_plan_collects_sorted_unique_normalized_files() {
    let (_temp, root, _dir, _store) = fixture();
    write(root.path(), "a.txt", "a-after");
    write(root.path(), "m.txt", "m-after");
    write(root.path(), "z.txt", "z-after");
    let absolute_a = root.path().join("a.txt").to_string_lossy().into_owned();
    let checkpoints = [
        checkpoint(
            0,
            vec![
                owned_file(
                    "z.txt",
                    Some("z-before"),
                    Some(true),
                    &digest(b"z-after"),
                    live_mode(),
                ),
                owned_file(
                    "./a.txt",
                    Some("a-before"),
                    Some(true),
                    &digest(b"a-after"),
                    live_mode(),
                ),
            ],
        ),
        checkpoint(
            1,
            vec![
                owned_file(
                    &absolute_a,
                    Some("a-middle"),
                    Some(true),
                    &digest(b"a-after"),
                    live_mode(),
                ),
                owned_file(
                    "m.txt",
                    Some("m-before"),
                    Some(true),
                    &digest(b"m-after"),
                    live_mode(),
                ),
            ],
        ),
    ];
    let store = load(root.path(), &_temp.path().join("paths.ckpt"), &checkpoints);

    let plan = store.prepare_rewind(0, RewindScope::Code, 1, 0, false);
    assert_eq!(plan.files, vec!["a.txt", "m.txt", "z.txt"]);
    assert_eq!(plan.file_count, 3);
    assert!(plan.can_files);
}

#[test]
fn precheck_reports_sorted_unsafe_missing_and_external_conflicts() {
    let (_temp, root, _dir, _store) = fixture();
    fs::create_dir(root.path().join("d-directory.txt")).unwrap();
    let checkpoints = [checkpoint(
        0,
        vec![
            owned_file(
                "d-directory.txt",
                Some("before"),
                Some(true),
                &digest(b"after"),
                0o644,
            ),
            owned_file(
                "c-external.txt",
                Some("before"),
                Some(true),
                &digest(b"after"),
                0o644,
            ),
            owned_file("b-missing.txt", None, Some(false), "", 0),
            owned_file("../a-unsafe.txt", Some("before"), Some(false), "", 0),
        ],
    )];
    let checkpoints = [Checkpoint {
        files: checkpoints[0]
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                if index == 2 {
                    FileSnap {
                        sha256: "missing-preimage".into(),
                        ..file.clone()
                    }
                } else {
                    file.clone()
                }
            })
            .collect(),
        ..checkpoints[0].clone()
    }];
    let store = load(
        root.path(),
        &_temp.path().join("conflicts.ckpt"),
        &checkpoints,
    );

    let conflicts = store.precheck_files(0);
    assert_eq!(
        conflicts
            .iter()
            .map(|conflict| conflict.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "../a-unsafe.txt",
            "b-missing.txt",
            "c-external.txt",
            "d-directory.txt"
        ]
    );
    assert_eq!(conflicts[0].reason, CONFLICT_PATH_UNSAFE);
    assert_eq!(conflicts[1].reason, CONFLICT_MISSING_PAYLOAD);
    assert_eq!(conflicts[1].checkpoint_sha, "missing-preimage");
    assert_eq!(conflicts[2].reason, CONFLICT_EXTERNAL_CHANGE);
    assert_eq!(conflicts[2].last_owned_sha, digest(b"after"));
    assert!(!conflicts[2].current_existed);
    assert!(conflicts[2].current_sha.is_empty());
    assert_eq!(conflicts[3].reason, CONFLICT_EXTERNAL_CHANGE);

    let plan = store.prepare_rewind(0, RewindScope::Code, 1, 0, false);
    assert_eq!(plan.conflicts, conflicts);
    assert!(!plan.can_files);
    assert_eq!(plan.disabled_reason, "file conflicts detected");
}

#[test]
fn earliest_revision_keeps_first_preimage_and_final_after_identity() {
    let (_temp, root, _dir, _store) = fixture();
    write(root.path(), "nested/a.txt", "latest");
    let checkpoints = [
        checkpoint(
            0,
            vec![owned_file(
                "nested/./a.txt",
                Some("original"),
                Some(true),
                &digest(b"middle"),
                0o640,
            )],
        ),
        checkpoint(
            1,
            vec![owned_file(
                "nested/a.txt",
                Some("middle"),
                Some(true),
                &digest(b"latest"),
                live_mode(),
            )],
        ),
    ];
    let store = load(
        root.path(),
        &_temp.path().join("earliest.ckpt"),
        &checkpoints,
    );

    let earliest = store.earliest_revisions(0);
    assert_eq!(earliest.len(), 1);
    let revision = &earliest["nested/a.txt"];
    assert_eq!(revision.content.as_deref(), Some("original"));
    assert_eq!(revision.sha256, digest(b"original"));
    assert_eq!(revision.mode, 0o644);
    assert_eq!(revision.after_sha256, digest(b"latest"));
    assert_eq!(revision.after_existed, Some(true));
    assert_eq!(revision.after_mode, live_mode());
}

#[test]
fn later_missing_after_identity_clears_proof_and_skips_file_legacy_conflict() {
    let (_temp, root, _dir, _store) = fixture();
    write(root.path(), "a.txt", "middle");
    let checkpoints = [
        checkpoint(
            0,
            vec![owned_file(
                "a.txt",
                Some("original"),
                Some(true),
                &digest(b"middle"),
                0o644,
            )],
        ),
        checkpoint(1, vec![owned_file("a.txt", Some("middle"), None, "", 0)]),
    ];
    let store = load(
        root.path(),
        &_temp.path().join("cleared.ckpt"),
        &checkpoints,
    );

    let revision = &store.earliest_revisions(0)["a.txt"];
    assert_eq!(revision.content.as_deref(), Some("original"));
    assert_eq!(revision.after_sha256, "");
    assert_eq!(revision.after_existed, None);
    assert_eq!(revision.after_mode, 0);
    assert!(store.precheck_files(0).is_empty());
}

#[test]
fn precheck_prefers_final_after_identity_over_earliest_preimage() {
    let (_temp, root, _dir, _store) = fixture();
    write(root.path(), "a.txt", "latest");
    let checkpoints = [
        checkpoint(
            0,
            vec![owned_file(
                "a.txt",
                Some("original"),
                Some(true),
                &digest(b"middle"),
                0o644,
            )],
        ),
        checkpoint(
            1,
            vec![owned_file(
                "a.txt",
                Some("middle"),
                Some(true),
                &digest(b"latest"),
                live_mode(),
            )],
        ),
    ];
    let store = load(
        root.path(),
        &_temp.path().join("prefer-after.ckpt"),
        &checkpoints,
    );

    assert!(store.precheck_files(0).is_empty());
    write(root.path(), "a.txt", "middle");
    let conflicts = store.precheck_files(0);
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].reason, CONFLICT_MANUAL_EDIT);
    assert_eq!(conflicts[0].checkpoint_sha, digest(b"original"));
    assert_eq!(conflicts[0].last_owned_sha, digest(b"latest"));
    assert_eq!(conflicts[0].current_sha, digest(b"middle"));
}
