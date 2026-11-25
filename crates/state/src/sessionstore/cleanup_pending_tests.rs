use super::*;
use std::time::SystemTime;

use super::super::branch::{BranchMeta, save_branch_meta};
use super::super::session_lease::try_acquire_session_lease;
use super::super::session_order::mark_cleanup_pending;

fn tempdir() -> tempfile::TempDir {
    crate::sessionstore::eventlog::test_support::tempdir()
}

const TOOL_ONLY_TRANSCRIPT: &str =
    "{\"role\":\"assistant\"}\n{\"role\":\"tool\",\"content\":\"ls\",\"tool_call_id\":\"tc-1\"}\n";
const USER_TRANSCRIPT: &str =
    "{\"role\":\"user\",\"content\":\"hi\"}\n{\"role\":\"assistant\",\"content\":\"hello\"}\n";

/// Write a transcript backdated by `age` (Go test helper
/// `writeAgedSession`).
fn write_aged_session(dir: &Path, name: &str, body: &str, age: Duration) -> String {
    let path = dir.join(name);
    fs::write(&path, body).expect("write transcript");
    let file = fs::File::options()
        .write(true)
        .open(&path)
        .expect("open for chtimes");
    file.set_modified(SystemTime::now() - age)
        .expect("set modified time");
    drop(file);
    path.to_string_lossy().into_owned()
}

#[test]
fn does_not_delete_published_fork() {
    let dir = tempdir();
    let path = dir.path().join("fork.jsonl");
    let path_text = path.to_string_lossy().into_owned();
    let dir_text = dir.path().to_string_lossy().into_owned();
    mark_cleanup_pending(&path_text, FORK_PENDING_OPERATION).expect("mark");
    let pending = list_cleanup_pending(&dir_text).expect("list");
    assert_eq!(pending.len(), 1);
    clear_cleanup_pending(&path_text).expect("clear");

    let mut calls = 0;
    let mut cb = |_item: CleanupPendingInfo| -> io::Result<()> {
        calls += 1;
        Ok(())
    };
    let handled = reconcile_pending(
        &dir_text,
        &pending[0],
        &mut Some(&mut cb as &mut dyn FnMut(CleanupPendingInfo) -> io::Result<()>),
    )
    .expect("reconcile published fork");
    assert!(handled);
    assert_eq!(calls, 0);
}

#[test]
fn rejects_mismatched_paths() {
    let dir = tempdir();
    let dir_text = dir.path().to_string_lossy().into_owned();
    let fork = dir.path().join("fork.jsonl");
    let other = dir.path().join("other.jsonl");
    let meta = CleanupPendingMeta {
        operation: FORK_PENDING_OPERATION.to_string(),
        ..CleanupPendingMeta::default()
    };
    let mut calls = 0;
    let mut cb = |_item: CleanupPendingInfo| -> io::Result<()> {
        calls += 1;
        Ok(())
    };

    // Marker belongs to another session in the same directory.
    let item = CleanupPendingInfo {
        session_path: fork.to_string_lossy().into_owned(),
        marker_path: naming::session_cleanup_pending(&other)
            .to_string_lossy()
            .into_owned(),
        meta: meta.clone(),
    };
    let result = reconcile_pending(
        &dir_text,
        &item,
        &mut Some(&mut cb as &mut dyn FnMut(CleanupPendingInfo) -> io::Result<()>),
    );
    assert!(result.is_err(), "mismatched marker must fail");

    // Session lives outside the reconciled directory.
    let other_dir = tempdir();
    let outside = other_dir.path().join("fork.jsonl");
    let item = CleanupPendingInfo {
        session_path: outside.to_string_lossy().into_owned(),
        marker_path: naming::session_cleanup_pending(&fork)
            .to_string_lossy()
            .into_owned(),
        meta: meta.clone(),
    };
    let result = reconcile_pending(
        &dir_text,
        &item,
        &mut Some(&mut cb as &mut dyn FnMut(CleanupPendingInfo) -> io::Result<()>),
    );
    assert!(result.is_err(), "outside session must fail");
    assert_eq!(calls, 0);
}

#[test]
fn rejects_marker_symlink() {
    let dir = tempdir();
    let target_dir = tempdir();
    let path = dir.path().join("fork.jsonl");
    let target = target_dir.path().join("outside-marker.json");
    fs::write(&target, "{\"operation\":\"fork\"}").expect("write marker target");
    let marker = naming::session_cleanup_pending(&path);
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&target, &marker).is_ok();
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&target, &marker).is_ok();
    if !linked {
        // No symlink privilege (Go: t.Skip).
        return;
    }
    let dir_text = dir.path().to_string_lossy().into_owned();
    let pending = list_cleanup_pending(&dir_text).expect("list");
    assert_eq!(pending.len(), 1);

    let mut calls = 0;
    let mut cb = |_item: CleanupPendingInfo| -> io::Result<()> {
        calls += 1;
        Ok(())
    };
    let result = reconcile_pending(
        &dir_text,
        &pending[0],
        &mut Some(&mut cb as &mut dyn FnMut(CleanupPendingInfo) -> io::Result<()>),
    );
    assert!(result.is_err(), "symlinked marker must fail");
    assert_eq!(calls, 0);
    assert!(target.exists(), "symlink target must survive");
}

#[test]
fn reclaimable_empty_sessions_picks_only_content_free_transcripts() {
    let dir = tempdir();
    let week = Duration::from_secs(7 * 24 * 60 * 60);
    let minute = Duration::from_secs(60);
    let empty = write_aged_session(
        dir.path(),
        "code-Reasonix__archive_202605201123.jsonl",
        TOOL_ONLY_TRANSCRIPT,
        week,
    );
    let blank = write_aged_session(
        dir.path(),
        "20260101-000000.000000000-model.jsonl",
        "",
        week,
    );
    write_aged_session(dir.path(), "kept-user.jsonl", USER_TRANSCRIPT, week);
    write_aged_session(
        dir.path(),
        "subagent-sub-k-202605171512.jsonl",
        TOOL_ONLY_TRANSCRIPT,
        week,
    );
    write_aged_session(dir.path(), "torn.jsonl", "{\"role\":\"assistant\"", week);
    write_aged_session(dir.path(), "fresh.jsonl", TOOL_ONLY_TRANSCRIPT, minute);

    let got = reclaimable_empty_sessions(
        &dir.path().to_string_lossy(),
        &chrono::Utc::now().fixed_offset(),
        EMPTY_SESSION_GRACE_PERIOD,
    )
    .expect("reclaimable listing");

    assert_eq!(got, vec![blank, empty]);
}

#[test]
fn reclaim_honors_meta_turns() {
    let dir = tempdir();
    let week = Duration::from_secs(7 * 24 * 60 * 60);
    let counted = write_aged_session(dir.path(), "counted.jsonl", TOOL_ONLY_TRANSCRIPT, week);
    save_branch_meta(
        Path::new(&counted),
        BranchMeta {
            turns: 3,
            ..BranchMeta::default()
        },
    )
    .expect("save meta");

    let got = reclaimable_empty_sessions(
        &dir.path().to_string_lossy(),
        &chrono::Utc::now().fixed_offset(),
        EMPTY_SESSION_GRACE_PERIOD,
    )
    .expect("reclaimable listing");

    assert!(got.is_empty(), "counted session must stay: {got:?}");
}

#[test]
fn reconcile_cleanup_pending_reclaims_empty_sessions() {
    let dir = tempdir();
    let week = Duration::from_secs(7 * 24 * 60 * 60);
    let empty = write_aged_session(dir.path(), "empty.jsonl", TOOL_ONLY_TRANSCRIPT, week);
    let kept = write_aged_session(dir.path(), "kept.jsonl", USER_TRANSCRIPT, week);

    let mut disposed: Vec<CleanupPendingInfo> = Vec::new();
    let mut cb = |item: CleanupPendingInfo| -> io::Result<()> {
        fs::remove_file(&item.session_path)?;
        disposed.push(item);
        Ok(())
    };
    reconcile_cleanup_pending(
        &dir.path().to_string_lossy(),
        Some(&mut cb as &mut dyn FnMut(CleanupPendingInfo) -> io::Result<()>),
    )
    .expect("reconcile");

    assert_eq!(disposed.len(), 1);
    assert_eq!(disposed[0].session_path, empty);
    assert_eq!(disposed[0].meta.operation, "delete");
    assert!(Path::new(&kept).exists(), "kept session must survive");
}

#[test]
fn reconcile_session_dir_deletes_under_the_removal_guard() {
    let dir = tempdir();
    let week = Duration::from_secs(7 * 24 * 60 * 60);
    let path = write_aged_session(dir.path(), "empty.jsonl", TOOL_ONLY_TRANSCRIPT, week);

    let mut held_during = false;
    let mut cb = |item: CleanupPendingInfo| -> io::Result<()> {
        match try_acquire_session_lease(&item.session_path) {
            Ok(lease) => lease.release(),
            Err(_) => held_during = true,
        }
        fs::remove_file(&item.session_path)?;
        Ok(())
    };
    reconcile_session_dir(
        &dir.path().to_string_lossy(),
        &mut Some(&mut cb as &mut dyn FnMut(CleanupPendingInfo) -> io::Result<()>),
    )
    .expect("reconcile session dir");

    assert!(held_during, "the removal guard must cover the callback");
    assert!(!Path::new(&path).exists());
}

#[test]
fn reclaim_empty_session_skips_one_opened_since_listing() {
    let dir = tempdir();
    let week = Duration::from_secs(7 * 24 * 60 * 60);
    let path = write_aged_session(dir.path(), "empty.jsonl", TOOL_ONLY_TRANSCRIPT, week);

    let _lease = try_acquire_session_lease(&path).expect("session lease");
    let mut calls = 0;
    let mut cb = |_item: CleanupPendingInfo| -> io::Result<()> {
        calls += 1;
        Ok(())
    };
    reclaim_empty_session(
        &path,
        &chrono::Utc::now().fixed_offset(),
        &mut Some(&mut cb as &mut dyn FnMut(CleanupPendingInfo) -> io::Result<()>),
    )
    .expect("reclaim");

    assert_eq!(calls, 0, "a held session must not be reclaimed");
}

#[test]
fn clear_cleanup_pending_keeps_malformed_v4_markers_unpublished() {
    let dir = tempdir();
    let sessions = dir.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    fs::create_dir_all(dir.path().join("sessions-v4")).expect("v4 root");
    let path = sessions.join("v4-0123456789abcdef0123456789abcdef.jsonl");
    let pending = naming::session_cleanup_pending(&path);
    let removed = naming::session_v4_removed_marker(&path);
    fs::write(&pending, b"{malformed").expect("pending marker");

    let error = clear_cleanup_pending(&path.to_string_lossy()).expect_err("malformed marker");
    assert!(error.get_ref().is_some_and(|source| {
        source
            .downcast_ref::<super::super::v4_import::ImportError>()
            .is_some()
    }));
    assert!(pending.exists(), "malformed cleanup marker must survive");
    assert!(!removed.exists(), "no tombstone may be published");
}

#[test]
fn clear_cleanup_pending_publishes_v4_tombstone_before_removing_marker() {
    let dir = tempdir();
    let sessions = dir.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    fs::create_dir_all(dir.path().join("sessions-v4")).expect("v4 root");
    let path = sessions.join("v4-0123456789abcdef0123456789abcdef.jsonl");
    let pending = naming::session_cleanup_pending(&path);
    let removed = naming::session_v4_removed_marker(&path);
    let raw = br#"{"operation":"delete","createdAt":1}"#;
    fs::write(&pending, raw).expect("pending marker");

    clear_cleanup_pending(&path.to_string_lossy()).expect("clear");
    assert!(!pending.exists());
    assert_eq!(fs::read(&removed).expect("tombstone"), raw);
}
