use std::fs::{self, File, FileTimes};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, FixedOffset};
use reasonix_core::filelock::try_acquire;
use serde_json::{Value, json};
use tempfile::TempDir;

use super::*;
use crate::sessionstore::eventlog::record::CreatedAt;
use crate::sessionstore::listing::SessionInfo;
use crate::sessionstore::sessionv4::test_support::{End, Fixture, ID, message};
use crate::sessionstore::sessionv4::{Session as V4Session, open, open_in};
use crate::store::naming::{
    is_v4_session_id, session_cleanup_pending, session_v4_removed_marker, v4_sessions_root,
};

fn project() -> TempDir {
    TempDir::new().expect("project")
}

fn source(root: &Path, id: &str, events: Vec<Value>) -> PathBuf {
    let mut fixture = Fixture::new(id);
    fixture.batch(events, End::Ended);
    let target = root.join("sessions-v4").join(id);
    fs::create_dir_all(target.parent().expect("source parent")).expect("source root");
    fs::rename(&fixture.dir, &target).expect("move source");
    target
}

fn user_source(root: &Path, id: &str, text: &str) -> PathBuf {
    source(root, id, vec![message("u", "user", text)])
}

fn set_modified(path: &Path, time: DateTime<FixedOffset>) {
    let time = SystemTime::UNIX_EPOCH
        + Duration::from_nanos(
            u64::try_from(time.timestamp_nanos_opt().expect("timestamp")).expect("nanos"),
        );
    let file = File::options()
        .write(true)
        .open(path)
        .expect("open timestamp file");
    file.set_times(FileTimes::new().set_modified(time))
        .expect("set mtime");
}

fn replay_count(calls: Arc<AtomicUsize>) -> LegacyReplay {
    Arc::new(move |session, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        read_legacy_transcript(session, &CancellationToken::new())
    })
}

fn await_snapshot<F>(
    service: &LegacyListings,
    dir: &Path,
    predicate: F,
) -> (Vec<SessionInfo>, ListingStatus)
where
    F: Fn(&[SessionInfo], &ListingStatus) -> bool,
{
    let (tx, rx) = mpsc::sync_channel(1);
    let _subscription = service.subscribe(move || {
        let _ = tx.try_send(());
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = service.snapshot(dir);
        if predicate(&snapshot.0, &snapshot.1) {
            return snapshot;
        }
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| panic!("listing did not settle: {:?}: {error}", snapshot.1));
    }
}

fn await_status<F>(service: &LegacyListings, dir: &Path, predicate: F) -> ListingStatus
where
    F: Fn(&ListingStatus) -> bool,
{
    await_snapshot(service, dir, |_, status| predicate(status)).1
}

#[test]
fn stale_cache_identity_rejects_size_mtime_and_manifest_drift() {
    let project = project();
    let dir = user_source(project.path(), ID, "one");
    let sessions = project.path().join("sessions");
    let old = open(&dir).expect("open old source");
    let mut cache = ListingCache::load(&sessions);
    cache.put(
        &old,
        Listing {
            title: "old".into(),
            ..Listing::default()
        },
    );
    assert_eq!(cache.get(&old).expect("fresh entry").title, "old");

    let mut fixture = Fixture::new(ID);
    fixture.batch(vec![message("u2", "user", "two")], End::Ended);
    fs::write(
        dir.join("events.frames"),
        fs::read(fixture.dir.join("events.frames")).expect("replacement frames"),
    )
    .expect("source append replacement");
    let resized = open(&dir).expect("open resized source");
    assert!(cache.get(&resized).is_none());

    cache.put(
        &resized,
        Listing {
            title: "resized".into(),
            ..Listing::default()
        },
    );
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).expect("manifest"))
            .expect("manifest json");
    manifest["createdAt"] = json!("2026-09-26T12:00:00Z");
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec(&manifest).expect("manifest json"),
    )
    .expect("rewrite manifest");
    let changed_manifest = open(&dir).expect("open changed manifest");
    assert!(cache.get(&changed_manifest).is_none());

    cache.put(&changed_manifest, Listing::default());
    let info = fs::metadata(dir.join("events.frames")).expect("frame metadata");
    let later = info.modified().expect("mtime") + Duration::from_secs(2);
    File::options()
        .write(true)
        .open(dir.join("events.frames"))
        .expect("open frames")
        .set_times(FileTimes::new().set_modified(later))
        .expect("change mtime");
    assert!(cache.get(&open(&dir).expect("open new mtime")).is_none());
}

#[test]
fn replay_rejects_a_source_that_changes_during_the_read() {
    let project = project();
    let dir = user_source(project.path(), ID, "before");
    let sessions = project.path().join("sessions");
    let resolver = LegacyResolver::new(&sessions);
    let source = open(&dir).expect("source");
    let changed_dir = dir.clone();
    let replay: LegacyReplay = Arc::new(move |session, _| {
        let mut fixture = Fixture::new(ID);
        fixture.batch(vec![message("u2", "user", "after")], End::Ended);
        fs::copy(
            fixture.dir.join("events.frames"),
            changed_dir.join("events.frames"),
        )
        .expect("change source during replay");
        read_legacy_transcript(session, &CancellationToken::new())
    });

    assert!(matches!(
        resolver.resolve(&source, replay),
        Err(LegacyListingError::SourceChanged)
    ));
    assert!(resolver.lookup(&source).is_none());
    let current = open(&dir).expect("current source");
    let calls = Arc::new(AtomicUsize::new(0));
    let listing = resolver
        .resolve(&current, replay_count(Arc::clone(&calls)))
        .expect("resolve changed source");
    assert_eq!(listing.preview, "after");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_conversation_is_replayed_once_and_reused_from_disk() {
    let project = project();
    let dir = user_source(project.path(), ID, "remember me");
    let sessions = project.path().join("sessions");
    let resolver = LegacyResolver::new(&sessions);
    let source = open(&dir).expect("source");
    let calls = Arc::new(AtomicUsize::new(0));
    let replay = replay_count(Arc::clone(&calls));
    let first = resolver
        .resolve(&source, Arc::clone(&replay))
        .expect("first resolve");
    let second = resolver
        .resolve(&source, Arc::clone(&replay))
        .expect("second resolve");
    assert_eq!(first, second);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let saved_source =
        open(&v4_sessions_root(&sessions).join(ID)).expect("open cache validation source");
    assert!(
        super::resolver::same_source(&source, &saved_source),
        "validation source identity must match"
    );
    resolver.flush(&sessions, true).expect("persist cache");
    let persisted = ListingCache::load(&sessions);
    assert_eq!(
        persisted.len(),
        1,
        "cache must publish the resolved listing"
    );
    assert_eq!(
        persisted.get(&source).expect("fresh persisted entry"),
        first
    );

    let log = dir.join("events.frames");
    let metadata = fs::metadata(&log).expect("log metadata");
    fs::write(&log, vec![0; metadata.len() as usize]).expect("poison log");
    File::options()
        .write(true)
        .open(&log)
        .expect("open poisoned log")
        .set_times(FileTimes::new().set_modified(metadata.modified().expect("mtime")))
        .expect("restore mtime");
    let reopened = open(&dir).expect("reopen poisoned source");
    assert!(
        super::resolver::same_source(&source, &reopened),
        "poisoning must preserve source identity"
    );

    let reloaded = LegacyResolver::new(&sessions);
    let never = Arc::new(|_: &V4Session, _: &CancellationToken| {
        panic!("a persisted listing must not replay the log")
    });
    let cached = reloaded
        .resolve(&source, never)
        .expect("disk cache resolve");
    assert_eq!(cached, first);
}

#[test]
fn failed_persistence_retains_one_shared_replay_for_retry() {
    let project = project();
    let dir = user_source(project.path(), ID, "shared");
    let sessions = project.path().join("sessions");
    let resolver = Arc::new(LegacyResolver::new(&sessions));
    let source = open(&dir).expect("source");
    let calls = Arc::new(AtomicUsize::new(0));
    let replay = replay_count(Arc::clone(&calls));
    let workers = (0..10)
        .map(|_| {
            let resolver = Arc::clone(&resolver);
            let source = source.clone();
            let replay = Arc::clone(&replay);
            std::thread::spawn(move || resolver.resolve(&source, replay))
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().expect("replay worker").expect("resolve");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let listing = resolver
        .resolve(&source, Arc::clone(&replay))
        .expect("cached resolve");

    fs::create_dir_all(sessions.join(CACHE_NAME)).expect("block cache destination");
    assert!(resolver.flush(&sessions, true).is_err());
    let never: LegacyReplay = Arc::new(|_: &V4Session, _: &CancellationToken| {
        panic!("a memory-resolved listing must survive a failed write")
    });
    assert_eq!(
        resolver
            .resolve(&source, Arc::clone(&never))
            .expect("memory cache"),
        listing
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    fs::remove_dir(sessions.join(CACHE_NAME)).expect("unblock cache destination");
    resolver.flush(&sessions, true).expect("forced retry");
    let reloaded = LegacyResolver::new(&sessions);
    assert_eq!(
        reloaded.resolve(&source, never).expect("disk cache"),
        listing
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cache_flush_merges_peers_and_revalidates_changed_sources() {
    let project = project();
    let first_dir = user_source(project.path(), ID, "first");
    let second_id = "1123456789abcdef0123456789abcdef";
    let second_dir = user_source(project.path(), second_id, "second");
    let sessions = project.path().join("sessions");
    let first = open(&first_dir).expect("first source");
    let second = open(&second_dir).expect("second source");
    let mut first_cache = ListingCache::load(&sessions);
    let mut second_cache = ListingCache::load(&sessions);
    first_cache.put(
        &first,
        Listing {
            title: "first".into(),
            ..Listing::default()
        },
    );
    second_cache.put(
        &second,
        Listing {
            title: "second".into(),
            ..Listing::default()
        },
    );

    let mut fixture = Fixture::new(ID);
    fixture.batch(vec![message("u2", "user", "changed")], End::Ended);
    fs::copy(
        fixture.dir.join("events.frames"),
        first_dir.join("events.frames"),
    )
    .expect("change first source");
    first_cache.save(&sessions).expect("flush stale entry");
    second_cache.save(&sessions).expect("flush peer entry");

    let persisted = ListingCache::load(&sessions);
    assert_eq!(persisted.len(), 1);
    assert!(persisted.get(&first).is_none());
    assert_eq!(persisted.get(&second).expect("peer entry").title, "second");
}

#[test]
fn cache_load_requires_the_declared_format_and_entries() {
    let project = project();
    let dir = user_source(project.path(), ID, "cached");
    let sessions = project.path().join("sessions");
    let mut cache = ListingCache::load(&sessions);
    cache.put(&open(&dir).expect("source"), Listing::default());
    cache.save(&sessions).expect("save cache");
    let path = sessions.join(CACHE_NAME);

    for raw in [json!({"entries": {}}), json!({"format": 1, "entries": {}})] {
        fs::write(&path, serde_json::to_vec(&raw).expect("cache json")).expect("rewrite cache");
        assert!(ListingCache::load(&sessions).is_empty());
    }
    for raw in [
        json!({"format": CACHE_FORMAT}),
        json!({"format": CACHE_FORMAT, "entries": null}),
    ] {
        let bytes = serde_json::to_vec(&raw).expect("cache json");
        assert!(serde_json::from_slice::<ListingCache>(&bytes).is_err());
        fs::write(&path, bytes).expect("rewrite cache");
        assert!(ListingCache::load(&sessions).is_empty());
    }
}

#[test]
fn cache_reuse_preserves_the_replayed_title() {
    let project = project();
    let dir = user_source(project.path(), ID, "titled");
    let sessions = project.path().join("sessions");
    let resolver = LegacyResolver::new(&sessions);
    let source = open(&dir).expect("source");
    let replay: LegacyReplay = Arc::new(|session, _| {
        let mut transcript = read_legacy_transcript(session, &CancellationToken::new())?;
        transcript.title = "  exact title  ".into();
        Ok(transcript)
    });

    let first = resolver
        .resolve(&source, Arc::clone(&replay))
        .expect("first resolve");
    let second = resolver.resolve(&source, replay).expect("cached resolve");
    assert_eq!(first, second);
    assert_eq!(second.title, "  exact title  ");
}

#[test]
fn cache_publication_uses_a_nonblocking_lock() {
    let project = project();
    let dir = user_source(project.path(), ID, "cached");
    let sessions = project.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    let mut cache = ListingCache::load(&sessions);
    cache.put(&open(&dir).expect("source"), Listing::default());
    let lock =
        try_acquire(&sessions.join(format!("{}.lock", CACHE_NAME))).expect("hold cache lock");
    let (tx, rx) = mpsc::channel();
    let worker_sessions = sessions.clone();
    std::thread::spawn(move || {
        tx.send(cache.save(&worker_sessions)).expect("report save");
    });
    let result = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("cache save waited for its lock");
    assert!(matches!(result, Err(LegacyListingError::CacheLockHeld)));
    drop(lock);
}

#[test]
fn flush_failure_backoff_is_bypassed_only_by_force() {
    let project = project();
    let dir = user_source(project.path(), ID, "cached");
    let sessions = project.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    let resolver = LegacyResolver::new(&sessions);
    let source = open(&dir).expect("source");
    let calls = Arc::new(AtomicUsize::new(0));
    resolver
        .resolve(&source, replay_count(Arc::clone(&calls)))
        .expect("resolve");
    let lock =
        try_acquire(&sessions.join(format!("{}.lock", CACHE_NAME))).expect("hold cache lock");
    assert!(matches!(
        resolver.flush(&sessions, false),
        Err(LegacyListingError::CacheLockHeld)
    ));
    drop(lock);
    assert!(matches!(
        resolver.flush(&sessions, false),
        Err(LegacyListingError::CacheLockHeld)
    ));
    assert!(!sessions.join(CACHE_NAME).exists());
    let never: LegacyReplay = Arc::new(|_: &V4Session, _: &CancellationToken| {
        panic!("backoff must retain the completed listing")
    });
    resolver
        .resolve(&source, never)
        .expect("memory cache during backoff");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    resolver.flush(&sessions, true).expect("forced retry");
    assert!(sessions.join(CACHE_NAME).is_file());
}

#[test]
fn discovery_maps_nonzero_rows_and_sorts_the_combined_list() {
    let project = project();
    let older = user_source(project.path(), ID, "older question");
    let newer_id = "1123456789abcdef0123456789abcdef";
    let newer = user_source(project.path(), newer_id, "newer question");
    let empty_id = "2123456789abcdef0123456789abcdef";
    source(
        project.path(),
        empty_id,
        vec![message("a", "assistant", "no user turn")],
    );
    set_modified(
        &older.join("events.frames"),
        DateTime::parse_from_rfc3339("2026-09-25T12:00:00Z").expect("time"),
    );
    set_modified(
        &newer.join("events.frames"),
        DateTime::parse_from_rfc3339("2026-09-26T12:00:00Z").expect("time"),
    );
    let sessions = project.path().join("sessions");

    let discovered = discover_legacy_sessions(&sessions).expect("discover");
    assert_eq!(discovered.sources.len(), 3);
    assert_eq!(discovered.failures, 0);
    let rows = with_v4_sessions(&sessions, Vec::new());
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].path,
        sessions
            .join(format!("v4-{newer_id}.jsonl"))
            .to_string_lossy()
    );
    assert_eq!(rows[0].preview, "newer question");
    assert_eq!(rows[0].turns, 1);
    assert!(rows[0].counts_known);
    assert_eq!(
        rows[0].created_at.get(),
        open(&newer).expect("source").manifest.created_at
    );
    assert_eq!(rows[0].last_activity_at, rows[0].mod_time);
    assert_eq!(rows[1].preview, "older question");
    assert!(rows.iter().all(|row| {
        is_v4_session_id(
            Path::new(&row.path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap()
                .trim_start_matches("v4-"),
        )
    }));
}

#[test]
fn complete_listing_appends_and_persists_legacy_rows() {
    let project = project();
    let dir = user_source(project.path(), ID, "remember me");
    let sessions = project.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");

    let first = crate::sessionstore::listing::list_sessions(&sessions).expect("first listing");
    assert_eq!(first.len(), 1);
    assert_eq!(
        first[0].path,
        sessions.join(format!("v4-{ID}.jsonl")).to_string_lossy()
    );
    assert_eq!(first[0].preview, "remember me");
    assert!(first[0].counts_known);
    assert_eq!(ListingCache::load(&sessions).len(), 1);

    let log = dir.join("events.frames");
    let metadata = fs::metadata(&log).expect("log metadata");
    fs::write(&log, vec![0; metadata.len() as usize]).expect("poison log");
    File::options()
        .write(true)
        .open(&log)
        .expect("open poisoned log")
        .set_times(FileTimes::new().set_modified(metadata.modified().expect("mtime")))
        .expect("restore mtime");

    let second = crate::sessionstore::listing::list_sessions(&sessions).expect("cached listing");
    assert_eq!(second, first);
}

#[test]
fn synchronous_listing_sorts_native_rows_when_legacy_discovery_fails() {
    let project = project();
    let sessions = project.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    fs::write(project.path().join("sessions-v4"), b"not a directory").expect("block legacy root");
    let early = CreatedAt::from_datetime(
        DateTime::parse_from_rfc3339("2026-09-25T12:00:00Z").expect("time"),
    );
    let late = CreatedAt::from_datetime(
        DateTime::parse_from_rfc3339("2026-09-26T12:00:00Z").expect("time"),
    );
    let rows = vec![
        SessionInfo {
            path: "b".into(),
            last_activity_at: early,
            ..SessionInfo::default()
        },
        SessionInfo {
            path: "a".into(),
            last_activity_at: late,
            ..SessionInfo::default()
        },
    ];

    let sorted = with_v4_sessions(&sessions, rows);
    assert_eq!(
        sorted
            .iter()
            .map(|row| row.path.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
}

#[test]
fn ready_zero_turn_rows_are_hidden_but_counts_are_complete() {
    let project = project();
    source(
        project.path(),
        ID,
        vec![message("a", "assistant", "answer only")],
    );
    let sessions = project.path().join("sessions");
    let service = LegacyListings::with_default_replay();
    let status = await_status(&service, &sessions, |status| status.counts_complete);
    assert_eq!(status.listing_state, ListingState::Ready);
    assert_eq!(status.pending_metadata, 0);
    assert_eq!(status.failed_metadata, 0);
    assert!(service.snapshot(&sessions).0.is_empty());
    service.close();
}

#[test]
fn sorting_is_stable_by_activity_descending_then_path() {
    let early = CreatedAt::from_datetime(
        DateTime::parse_from_rfc3339("2026-09-25T12:00:00Z").expect("time"),
    );
    let late = CreatedAt::from_datetime(
        DateTime::parse_from_rfc3339("2026-09-26T12:00:00Z").expect("time"),
    );
    let mut rows = vec![
        SessionInfo {
            path: "b".into(),
            last_activity_at: early,
            preview: "b".into(),
            ..SessionInfo::default()
        },
        SessionInfo {
            path: "z".into(),
            last_activity_at: late,
            preview: "late".into(),
            ..SessionInfo::default()
        },
        SessionInfo {
            path: "a".into(),
            last_activity_at: early,
            preview: "first a".into(),
            ..SessionInfo::default()
        },
        SessionInfo {
            path: "a".into(),
            last_activity_at: early,
            preview: "second a".into(),
            ..SessionInfo::default()
        },
    ];
    sort_legacy_rows(&mut rows);
    let previews: Vec<_> = rows.iter().map(|row| row.preview.as_str()).collect();
    assert_eq!(previews, ["late", "first a", "second a", "b"]);
}

#[test]
fn snapshot_returns_pending_rows_without_waiting_for_replay() {
    let project = project();
    user_source(project.path(), ID, "hello");
    let sessions = project.path().join("sessions");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let service = LegacyListings::new(move |session, token| {
        entered_tx.send(()).expect("signal replay");
        while release_rx
            .lock()
            .expect("release receiver")
            .recv_timeout(Duration::from_millis(10))
            .is_err()
        {
            if token.is_cancelled() {
                return Err(LegacyListingError::Cancelled);
            }
        }
        read_legacy_transcript(session, token)
    });
    service.snapshot(&sessions);
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("replay started");
    let snapshot_service = service.clone();
    let snapshot_dir = sessions.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        tx.send(snapshot_service.snapshot(&snapshot_dir))
            .expect("snapshot")
    });
    let (rows, status) = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("snapshot waited for replay");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].metadata_state, "pending");
    assert!(!rows[0].counts_known);
    assert_eq!(status.pending_metadata, 1);
    assert!(!status.counts_complete);
    release_tx.send(()).expect("release replay");
    let status = await_status(&service, &sessions, |status| status.counts_complete);
    assert_eq!(status.listing_state, ListingState::Ready);
    let rows = service.snapshot(&sessions).0;
    assert_eq!(rows[0].metadata_state, "ready");
    assert_eq!(rows[0].preview, "hello");
    service.close();
}

#[test]
fn failed_rows_are_visible_and_retry_replays_them() {
    let project = project();
    user_source(project.path(), ID, "retry me");
    let sessions = project.path().join("sessions");
    let calls = Arc::new(AtomicUsize::new(0));
    let replay_calls = Arc::clone(&calls);
    let service = LegacyListings::new(move |session, token| {
        if replay_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(LegacyListingError::Io {
                path: session.dir.clone(),
                source: Arc::new(io::Error::other("injected replay failure")),
            });
        }
        read_legacy_transcript(session, token)
    });
    let status = await_status(&service, &sessions, |status| status.failed_metadata == 1);
    assert_eq!(status.listing_state, ListingState::Degraded);
    let rows = service.snapshot(&sessions).0;
    assert_eq!(rows[0].metadata_state, "error");
    service.retry(&sessions);
    let status = await_status(&service, &sessions, |status| status.counts_complete);
    assert_eq!(status.listing_state, ListingState::Ready);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    service.close();
}

#[test]
fn rescans_remove_deleted_and_tombstoned_legacy_rows() {
    let project = project();
    user_source(project.path(), ID, "removed");
    let deleted_id = "1123456789abcdef0123456789abcdef";
    let deleted = user_source(project.path(), deleted_id, "deleted");
    let sessions = project.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    let service = LegacyListings::with_default_replay();
    await_snapshot(&service, &sessions, |rows, status| {
        rows.len() == 2 && status.counts_complete
    });

    let removed_path = sessions.join(format!("v4-{ID}.jsonl"));
    fs::write(v4_removal_path(&removed_path), b"removed").expect("publish tombstone");
    fs::remove_dir_all(deleted).expect("delete source");
    assert!(
        service
            .snapshot(&sessions)
            .0
            .iter()
            .all(|row| row.path != removed_path.to_string_lossy())
    );

    service.retry(&sessions);
    let (rows, status) = await_snapshot(&service, &sessions, |rows, status| {
        rows.is_empty() && status.counts_complete
    });
    assert!(rows.is_empty());
    assert_eq!(status.listing_state, ListingState::Ready);
    assert_eq!(status.pending_metadata, 0);
    assert_eq!(status.failed_metadata, 0);
    service.close();
}

#[test]
fn close_cancels_a_waiting_replay_without_waiting_for_backlog() {
    let project = project();
    for i in 0..3 {
        user_source(project.path(), &format!("{i:024x}"), "hello");
    }
    let sessions = project.path().join("sessions");
    let service = LegacyListings::new(|_, token| {
        token.wait();
        Err(LegacyListingError::Cancelled)
    });
    service.snapshot(&sessions);
    let (tx, rx) = mpsc::channel();
    let closer = service.clone();
    std::thread::spawn(move || {
        closer.close();
        tx.send(()).expect("report close");
    });
    rx.recv_timeout(Duration::from_secs(1))
        .expect("close waited for replay");
}

#[test]
fn close_waits_for_the_current_replay_and_flushes_completed_rows() {
    let project = project();
    let first_id = "ffffffffffffffffffffffffffff";
    let second_id = "1123456789abcdef0123456789abcdef";
    let first = user_source(project.path(), first_id, "completed");
    let second = user_source(project.path(), second_id, "cancelled");
    set_modified(
        &first.join("events.frames"),
        DateTime::parse_from_rfc3339("2026-10-02T12:00:00Z").expect("time"),
    );
    set_modified(
        &second.join("events.frames"),
        DateTime::parse_from_rfc3339("2026-10-01T12:00:00Z").expect("time"),
    );
    let sessions = project.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    let calls = Arc::new(AtomicUsize::new(0));
    let replay_calls = Arc::clone(&calls);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (cancelled_tx, cancelled_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let service = LegacyListings::new(move |session, token| {
        if replay_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return read_legacy_transcript(session, token);
        }
        entered_tx.send(()).expect("signal blocked replay");
        token.wait();
        cancelled_tx.send(()).expect("signal cancellation");
        release_rx
            .lock()
            .expect("release receiver")
            .recv()
            .expect("release cancelled replay");
        Err(LegacyListingError::Cancelled)
    });
    service.snapshot(&sessions);
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("second replay did not start");

    let (closed_tx, closed_rx) = mpsc::channel();
    let closer = service.clone();
    std::thread::spawn(move || {
        closer.close();
        closed_tx.send(()).expect("report close");
    });
    cancelled_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("close did not cancel replay");
    assert!(
        closed_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "close returned before the worker finished"
    );
    release_tx.send(()).expect("release cancelled replay");
    closed_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("close did not finish");
    assert_eq!(ListingCache::load(&sessions).len(), 1);
}

#[test]
fn completed_batches_flush_before_the_next_replay_blocks() {
    let project = project();
    for i in 0..34 {
        user_source(
            project.path(),
            &format!("{i:024x}"),
            &format!("question {i}"),
        );
    }
    let sessions = project.path().join("sessions");
    let calls = Arc::new(AtomicUsize::new(0));
    let replay_calls = Arc::clone(&calls);
    let (entered_tx, entered_rx) = mpsc::channel();
    let service = LegacyListings::new(move |session, token| {
        if replay_calls.fetch_add(1, Ordering::SeqCst) == 32 {
            entered_tx.send(()).expect("signal blocked replay");
            token.wait();
            return Err(LegacyListingError::Cancelled);
        }
        read_legacy_transcript(session, token)
    });
    service.snapshot(&sessions);
    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("batch did not reach replay 33");
    assert_eq!(ListingCache::load(&sessions).entries.len(), 32);
    let (rows, status) = service.snapshot(&sessions);
    assert_eq!(rows.len(), 34);
    assert_eq!(status.pending_metadata, 2);
    assert!(!status.counts_complete);
    service.close();
}

#[test]
fn resolver_instances_are_shared_per_cleaned_directory() {
    let project = project();
    let sessions = project.path().join("sessions");
    let first = resolver_for(&sessions);
    let second = resolver_for(&sessions.join("."));
    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn removal_paths_accept_only_v4_imports_in_sessions_directories() {
    let project = project();
    let sessions = project.path().join("sessions");
    let path = sessions.join(format!("v4-{ID}.jsonl"));
    assert_eq!(v4_removal_path(&path), session_v4_removed_marker(&path));
    assert_eq!(
        v4_removal_path(&path),
        sessions.join(format!("v4-{ID}.v4-removed"))
    );
    for invalid in [
        project.path().join(format!("v4-{ID}.jsonl")),
        sessions.join(format!("{ID}.jsonl")),
        sessions.join("v4-not-an-id.jsonl"),
        sessions.join(format!("v4-{ID}.jsonl.meta")),
    ] {
        assert!(v4_removal_path(&invalid).as_os_str().is_empty());
    }
    assert!(!is_v4_session_removed(&path));
}

#[test]
fn removal_publication_failure_preserves_cleanup_for_retry() {
    let project = project();
    source(project.path(), ID, vec![message("u", "user", "hello")]);
    let sessions = project.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    let path = sessions.join(format!("v4-{ID}.jsonl"));
    crate::sessionstore::session_order::mark_cleanup_pending(&path.to_string_lossy(), "remove")
        .expect("mark cleanup");
    let pending = session_cleanup_pending(&path);
    let bytes = fs::read(&pending).expect("cleanup bytes");
    let removed = v4_removal_path(&path);
    fs::create_dir_all(removed.join("held")).expect("block tombstone publication");
    assert!(preserve_v4_removal(&path).is_err());
    assert_eq!(fs::read(&pending).expect("cleanup survived"), bytes);
    assert!(
        fs::metadata(&removed)
            .expect("blocking directory remains")
            .is_dir()
    );
    fs::remove_dir_all(&removed).expect("unblock tombstone");
    preserve_v4_removal(&path).expect("retry tombstone publication");
    assert_eq!(
        fs::read(v4_removal_path(&path)).expect("published marker"),
        bytes
    );
    assert!(is_v4_session_removed(&path));
    assert_eq!(
        fs::read(&pending).expect("caller still owns cleanup"),
        bytes
    );
}

#[test]
fn removal_markers_are_confined_to_the_sessions_directory() {
    let project = project();
    let sessions = project.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    let path = sessions.join(format!("v4-{ID}.jsonl"));
    let outside = project.path().join("outside");
    #[cfg(unix)]
    {
        fs::write(&outside, b"external").expect("outside marker");
        std::os::unix::fs::symlink(&outside, v4_removal_path(&path)).expect("removal symlink");
        assert!(!is_v4_session_removed(&path));
        fs::remove_file(v4_removal_path(&path)).expect("remove symlink");
        std::os::unix::fs::symlink(&outside, session_cleanup_pending(&path))
            .expect("pending symlink");
        assert!(preserve_v4_removal(&path).is_err());
        assert!(!v4_removal_path(&path).exists());
    }
    #[cfg(windows)]
    {
        fs::create_dir_all(&outside).expect("outside directory");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(v4_removal_path(&path))
            .arg(&outside)
            .output()
            .expect("create removal junction");
        assert!(status.status.success(), "junction failed: {status:?}");
        assert!(!is_v4_session_removed(&path));
        fs::remove_dir(v4_removal_path(&path)).expect("remove junction");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(session_cleanup_pending(&path))
            .arg(&outside)
            .output()
            .expect("create pending junction");
        assert!(status.status.success(), "junction failed: {status:?}");
        assert!(preserve_v4_removal(&path).is_err());
        assert!(!v4_removal_path(&path).exists());
    }
}

#[test]
fn discovery_skips_imported_removed_and_invalid_sources() {
    let project = project();
    let visible = user_source(project.path(), ID, "visible");
    let imported_id = "1123456789abcdef0123456789abcdef";
    let imported = user_source(project.path(), imported_id, "imported");
    let removed_id = "2123456789abcdef0123456789abcdef";
    let removed = user_source(project.path(), removed_id, "removed");
    let sessions = project.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    fs::write(
        sessions.join(format!("v4-{imported_id}.jsonl")),
        b"imported",
    )
    .expect("native import");
    preserve_v4_removal(&sessions.join(format!("v4-{removed_id}.jsonl"))).expect("no pending");
    fs::write(
        v4_removal_path(&sessions.join(format!("v4-{removed_id}.jsonl"))),
        b"removed",
    )
    .expect("tombstone");
    fs::create_dir_all(project.path().join("sessions-v4/not-an-id")).expect("invalid id");
    let discovery = discover_legacy_sessions(&sessions).expect("discovery");
    assert_eq!(discovery.sources.len(), 1);
    assert_eq!(
        fs::canonicalize(&discovery.sources[0].dir).expect("canonical discovered source"),
        fs::canonicalize(&visible).expect("canonical fixture source")
    );
    assert_eq!(discovery.failures, 0);
    assert_eq!(
        v4_sessions_root(&sessions),
        project.path().join("sessions-v4")
    );
    assert_eq!(
        open_in(&project.path().join("sessions-v4"), imported_id)
            .expect("imported source")
            .log_size,
        open(&imported).expect("source").log_size
    );
    assert_eq!(
        open(&removed).expect("removed source").manifest.session_id,
        removed_id
    );
}
