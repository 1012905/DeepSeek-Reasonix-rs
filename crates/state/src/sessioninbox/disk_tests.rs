use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use cap_std::ambient_authority;
use cap_std::fs::Dir;

use super::disk::{Store, acquire_transaction_lock_deadline};
use super::disk_fs::read_regular_file_in;
use super::{
    DEFAULT_MAX_ITEM_BYTES, DEFAULT_MAX_ITEMS, DEFAULT_MAX_TOTAL_BYTES, Error, Limits,
    SCHEMA_VERSION, decode_manifest, empty_manifest, process_run_id, remove_dir, sha256_hex,
};
use crate::store::naming::session_inbox_dir;

pub(super) mod support {
    use std::fs;
    use std::path::{Path, PathBuf};

    use reasonix_contract::gojson::marshal;

    use super::super::{
        InboxIntent, InboxItemMeta, InboxState, Manifest, PromptEnvelope, StructuredInvocation,
    };
    use crate::sessionstore::eventlog::record::CreatedAt;
    use crate::store::naming::session_inbox_dir;

    pub(in crate::sessioninbox) fn inbox(root: &Path, name: &str) -> PathBuf {
        session_inbox_dir(&root.join(format!("{name}.jsonl")))
    }

    pub(in crate::sessioninbox) fn manifest_bytes(manifest: &Manifest) -> Vec<u8> {
        let mut bytes = marshal(manifest).unwrap().into_bytes();
        bytes.push(b'\n');
        bytes
    }

    pub(in crate::sessioninbox) fn write_manifest(root: &Path, name: &str, manifest: &Manifest) {
        let dir = inbox(root, name);
        fs::create_dir_all(dir.join("blobs")).unwrap();
        fs::write(dir.join("manifest.json"), manifest_bytes(manifest)).unwrap();
    }

    pub(in crate::sessioninbox) fn write_blob(root: &Path, name: &str, stem: &str, bytes: &[u8]) {
        let dir = inbox(root, name);
        fs::create_dir_all(dir.join("blobs")).unwrap();
        fs::write(dir.join("blobs").join(format!("{stem}.json")), bytes).unwrap();
    }

    pub(in crate::sessioninbox) fn write_envelope(
        root: &Path,
        name: &str,
        stem: &str,
        env: &PromptEnvelope,
    ) {
        write_blob(root, name, stem, marshal(env).unwrap().as_bytes());
    }

    pub(in crate::sessioninbox) fn item(id: &str, state: &str) -> InboxItemMeta {
        InboxItemMeta {
            id: id.to_owned(),
            intent: InboxIntent::new("followup"),
            state: InboxState::new(state),
            revision: 1,
            blob_name: id.to_owned(),
            created_at: CreatedAt::now(),
            updated_at: CreatedAt::now(),
            preview: id.to_owned(),
            ..InboxItemMeta::default()
        }
    }

    pub(in crate::sessioninbox) fn invocation(name: &str, offset: i64) -> StructuredInvocation {
        StructuredInvocation {
            name: name.to_owned(),
            offset,
            display: format!("display {name}"),
            ..StructuredInvocation::default()
        }
    }
}

#[test]
fn open_creates_private_layout_and_empty_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let session = temp.path().join("session.jsonl");
    let padded = format!("  {}  ", session.display());
    let store = Store::open(Path::new(&padded), Limits::default()).unwrap();
    let dir = session_inbox_dir(&session);

    assert_eq!(store.dir(), dir);
    assert_eq!(store.session_path(), session);
    assert!(dir.is_dir());
    assert!(!dir.join("manifest.json").exists());
    assert!(!dir.join("blobs").exists());
    assert!(dir.join("transaction.lock").is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    let snap = store.snapshot();
    assert_eq!(snap.schema_version, SCHEMA_VERSION);
    assert_eq!(snap.revision, 0);
    assert_eq!(snap.run_id, process_run_id());
    assert_eq!(snap.items, None);
    assert!(!snap.paused);
    assert!(!snap.readonly);
    assert_eq!(snap.capacity.max_items, DEFAULT_MAX_ITEMS);
    assert_eq!(snap.capacity.max_item_bytes, DEFAULT_MAX_ITEM_BYTES);
    assert_eq!(snap.capacity.max_bytes, DEFAULT_MAX_TOTAL_BYTES);
}

#[test]
fn open_rejects_an_empty_session_path() {
    let err = Store::open(Path::new(" \t "), Limits::default()).unwrap_err();
    assert!(matches!(err, Error::EmptySessionPath));
}

#[test]
fn empty_snapshot_serializes_items_as_null() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let value = serde_json::to_value(store.snapshot()).unwrap();
    assert_eq!(value["items"], serde_json::Value::Null);
}

#[cfg(unix)]
#[test]
fn open_preserves_non_utf8_session_path_units() {
    use std::ffi::{OsStr, OsString};
    use std::os::unix::ffi::OsStrExt;

    let temp = tempfile::tempdir().unwrap();
    let mut session = temp.path().as_os_str().to_os_string();
    session.push("/session-");
    session.push(OsStr::from_bytes(b"\xff"));
    session.push(".jsonl");
    let mut padded = OsString::from(" \t");
    padded.push(&session);
    padded.push(" \n");

    let store = Store::open(Path::new(&padded), Limits::default()).unwrap();
    assert_eq!(store.session_path().as_os_str(), session.as_os_str());
    assert_eq!(store.dir(), session_inbox_dir(Path::new(&session)));
}

#[cfg(windows)]
#[test]
fn windows_path_trimming_preserves_unpaired_surrogates() {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    let input = OsString::from_wide(&[0x20, 0xd800, b'a' as u16, 0x20]);
    let trimmed = super::disk::trim_session_path(&input);
    assert_eq!(
        trimmed.encode_wide().collect::<Vec<_>>(),
        vec![0xd800, b'a' as u16]
    );

    let input = OsString::from_wide(&[0x20, b'a' as u16, 0xdc00, 0x20]);
    let trimmed = super::disk::trim_session_path(&input);
    assert_eq!(
        trimmed.encode_wide().collect::<Vec<_>>(),
        vec![b'a' as u16, 0xdc00]
    );
}

#[test]
fn transaction_lock_has_a_short_deadline_and_raii_release() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let first = acquire_transaction_lock_deadline(
        &store.dir(),
        Instant::now() + Duration::from_millis(40),
        Duration::from_millis(5),
    )
    .unwrap();

    let err = acquire_transaction_lock_deadline(
        &store.dir(),
        Instant::now() + Duration::from_millis(30),
        Duration::from_millis(5),
    )
    .unwrap_err();
    assert!(matches!(err, Error::LockTimeout { .. }));
    drop(first);

    let second = acquire_transaction_lock_deadline(
        &store.dir(),
        Instant::now() + Duration::from_secs(1),
        Duration::from_millis(5),
    )
    .unwrap();
    drop(second);
}

#[test]
fn rebind_retains_the_new_binding_when_loading_fails() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("first.jsonl"), Limits::default()).unwrap();
    fs::write(temp.path().join("blocked"), b"not a directory").unwrap();
    let target = temp.path().join("blocked").join("second.jsonl");

    assert!(store.rebind(&target).is_err());
    assert_eq!(store.session_path(), target);
    assert_eq!(store.dir(), session_inbox_dir(&target));
}

#[test]
fn close_seals_mutations_but_snapshot_still_refreshes() {
    let temp = tempfile::tempdir().unwrap();
    let session = temp.path().join("session.jsonl");
    let store = Store::open(&session, Limits::default()).unwrap();
    let mut next = empty_manifest(process_run_id());
    next.revision = 2;
    {
        let mut state = store.state.lock().unwrap();
        store.commit_manifest_locked(&mut state, next).unwrap();
    }
    store.close();

    let state = store.state.lock().unwrap();
    assert!(matches!(store.mutable_locked(&state), Err(Error::Closed)));
    drop(state);
    assert!(matches!(store.rebind(&session), Err(Error::Closed)));

    let mut external = empty_manifest("external");
    external.revision = 7;
    support::write_manifest(temp.path(), "session", &external);
    let snap = store.snapshot();
    assert_eq!(snap.revision, 7);
    assert_eq!(snap.run_id, process_run_id());
}

#[test]
fn transaction_free_refresh_rejects_a_foreign_run_without_adopting_it() {
    let temp = tempfile::tempdir().unwrap();
    let session = temp.path().join("session.jsonl");
    let store = Store::open(&session, Limits::default()).unwrap();
    let mut next = empty_manifest(process_run_id());
    next.revision = 4;
    {
        let mut state = store.state.lock().unwrap();
        store.commit_manifest_locked(&mut state, next).unwrap();
    }
    let mut external = empty_manifest("foreign");
    external.revision = 9;
    support::write_manifest(temp.path(), "session", &external);

    let mut state = store.state.lock().unwrap();
    assert!(matches!(
        store.read_manifest_without_transaction(&mut state),
        Err(Error::ForeignRun)
    ));
    assert_eq!(state.queue.manifest.revision, 5);
    assert_eq!(state.queue.manifest.run_id, process_run_id());
}

#[test]
fn snapshot_keeps_the_last_state_when_refresh_fails() {
    let temp = tempfile::tempdir().unwrap();
    let session = temp.path().join("session.jsonl");
    let store = Store::open(&session, Limits::default()).unwrap();
    let mut next = empty_manifest(process_run_id());
    next.revision = 4;
    {
        let mut state = store.state.lock().unwrap();
        store.commit_manifest_locked(&mut state, next).unwrap();
    }
    let manifest = store.dir().join("manifest.json");
    fs::remove_file(&manifest).unwrap();
    fs::create_dir(&manifest).unwrap();

    let snap = store.snapshot();
    assert_eq!(snap.revision, 5);
    assert_eq!(snap.items, None);
}

#[test]
fn regular_reads_are_bounded_and_refuse_non_regular_files() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("body");
    fs::write(&file, b"12345").unwrap();
    assert!(matches!(
        read_regular_file(&file, 4),
        Err(Error::TooLarge { .. })
    ));
    assert_eq!(read_regular_file(&file, 5).unwrap(), b"12345");
    assert!(matches!(
        read_regular_file(temp.path(), 5),
        Err(Error::NotRegularFile { .. })
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let link = temp.path().join("link");
        symlink(&file, &link).unwrap();
        assert!(matches!(
            read_regular_file(&link, 5),
            Err(Error::NotRegularFile { .. })
        ));
    }
}

#[test]
fn blob_reads_check_the_checksum_before_decoding_json() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let bytes = br#"{"displayText":"hello"}"#;
    store.write_blob_locked("body", bytes).unwrap();

    let err = store.read_blob_locked("body", &"0".repeat(64)).unwrap_err();
    assert!(matches!(err, Error::ChecksumMismatch));
    let env = store.read_blob_locked("body", &sha256_hex(bytes)).unwrap();
    assert_eq!(env.display_text, "hello");

    store.write_blob_locked("broken", b"not json").unwrap();
    assert!(matches!(
        store.read_blob_locked("broken", &sha256_hex(b"not json")),
        Err(Error::Json(_))
    ));
    assert!(matches!(
        store.read_blob_locked("../escape", ""),
        Err(Error::InvalidItemPath)
    ));
}

#[test]
fn blob_writes_are_private_and_reject_symlinked_roots() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    store.write_blob_locked("body", b"{}").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        assert_eq!(
            fs::metadata(store.dir().join("blobs"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.dir().join("blobs/body.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::remove_dir_all(store.dir().join("blobs")).unwrap();
        symlink(&outside, store.dir().join("blobs")).unwrap();
        assert!(store.write_blob_locked("body", b"{}").is_err());
        assert!(store.read_blob_locked("body", "").is_err());
    }
}

#[test]
fn commit_validates_stamps_and_publishes_before_memory_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let mut next = empty_manifest("stale");
    next.schema_version = 1;
    next.revision = 41;
    let mut html = support::item("html", "queued");
    html.preview = "<&>".to_owned();
    next.items.push(html);
    {
        let mut state = store.state.lock().unwrap();
        store
            .commit_manifest_locked(&mut state, next.clone())
            .unwrap();
        assert_eq!(state.queue.manifest.revision, 42);
        assert_eq!(state.queue.manifest.run_id, process_run_id());
    }
    let bytes = fs::read(store.dir().join("manifest.json")).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.ends_with('\n'));
    assert!(text.contains("\n  \"schemaVersion\": 3,"));
    assert!(text.contains("\"preview\": \"\\u003c\\u0026\\u003e\""));
    let disk = decode_manifest(text.as_bytes()).unwrap();
    assert_eq!(disk.schema_version, SCHEMA_VERSION);
    assert_eq!(disk.run_id, process_run_id());
    assert_eq!(disk.revision, 42);
    assert!(!disk.updated_at.is_zero());

    let mut invalid = disk.clone();
    invalid.items.push(support::item("same", "queued"));
    invalid.items.push(support::item("same", "queued"));
    let manifest_path = store.dir().join("manifest.json");
    let mut state = store.state.lock().unwrap();
    assert!(matches!(
        store.commit_manifest_locked(&mut state, invalid),
        Err(Error::DuplicateItemId)
    ));
    assert_eq!(fs::read(&manifest_path).unwrap(), text.as_bytes());
}

#[test]
fn loaded_manifest_commit_failure_retains_stamped_memory_but_clone_failure_does_not() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let manifest_path = store.dir().join("manifest.json");
    fs::create_dir(&manifest_path).unwrap();
    let mut state = store.state.lock().unwrap();
    state.queue.manifest.revision = 7;

    let mut cloned = empty_manifest("stale");
    cloned.revision = 41;
    assert!(store.commit_manifest_locked(&mut state, cloned).is_err());
    assert_eq!(state.queue.manifest.revision, 7);

    state.queue.manifest = empty_manifest("stale");
    state.queue.manifest.revision = 41;
    assert!(store.commit_current_manifest_locked(&mut state).is_err());
    assert_eq!(state.queue.manifest.revision, 42);
    assert_eq!(state.queue.manifest.schema_version, SCHEMA_VERSION);
    assert_eq!(state.queue.manifest.run_id, process_run_id());
}

#[test]
fn commit_refuses_to_overwrite_unrecovered_state() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let manifest_path = store.dir().join("manifest.json");
    let mut state = store.state.lock().unwrap();
    state.queue.unrecovered = true;
    assert!(matches!(
        store.commit_manifest_locked(&mut state, empty_manifest(process_run_id())),
        Err(Error::Unrecovered)
    ));
    assert!(!manifest_path.exists());
}

#[test]
fn change_listeners_run_asynchronously_and_can_reenter() {
    let temp = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap());
    let (tx, rx) = mpsc::channel();
    let callback_store = Arc::clone(&store);
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_calls = Arc::clone(&calls);
    store.on_change(move |snap| {
        assert_eq!(
            callback_store.dir(),
            callback_store.session_path().with_extension("inbox")
        );
        assert_eq!(snap.revision, 3);
        callback_calls.fetch_add(1, Ordering::SeqCst);
        tx.send(()).unwrap();
    });

    let state = store.state.lock().unwrap();
    let mut snapshot = store.snapshot_locked(&state);
    snapshot.revision = 3;
    store.notify_locked(&state, snapshot);
    drop(state);
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn remove_dir_removes_only_the_inbox_and_tolerates_absence() {
    let temp = tempfile::tempdir().unwrap();
    let session = temp.path().join("session.jsonl");
    fs::write(&session, b"transcript").unwrap();
    let store = Store::open(&session, Limits::default()).unwrap();
    store.write_blob_locked("body", b"{}").unwrap();

    remove_dir(&session).unwrap();
    assert!(!store.dir().exists());
    assert_eq!(fs::read(&session).unwrap(), b"transcript");
    remove_dir(&session).unwrap();
}

#[test]
fn remove_dir_uses_the_trimmed_session_path() {
    let temp = tempfile::tempdir().unwrap();
    let session = temp.path().join("session.jsonl");
    let store = Store::open(&session, Limits::default()).unwrap();
    let padded = format!(" \t{} \n", session.display());

    remove_dir(Path::new(&padded)).unwrap();
    assert!(!store.dir().exists());
}

#[cfg(unix)]
#[test]
fn remove_dir_refuses_a_symlink_inbox_root_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let session = temp.path().join("session.jsonl");
    let inbox = session_inbox_dir(&session);
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"keep").unwrap();
    symlink(&outside, &inbox).unwrap();

    assert!(remove_dir(&session).is_err());
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
    assert!(
        fs::symlink_metadata(&inbox)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn inbox_root_symlinks_are_refused() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let session = temp.path().join("session.jsonl");
    let inbox = session_inbox_dir(&session);
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, &inbox).unwrap();
    assert!(Store::open(&session, Limits::default()).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_session_parents_remain_supported() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let real_parent = temp.path().join("real");
    let linked_parent = temp.path().join("linked");
    fs::create_dir(&real_parent).unwrap();
    symlink(&real_parent, &linked_parent).unwrap();
    let session = linked_parent.join("session.jsonl");

    let store = Store::open(&session, Limits::default()).unwrap();
    assert_eq!(store.dir(), session_inbox_dir(&session));
    assert!(real_parent.join("session.inbox").is_dir());
}

#[cfg(windows)]
#[test]
fn windows_junction_inbox_roots_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let session = temp.path().join("session.jsonl");
    let inbox = session_inbox_dir(&session);
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"keep").unwrap();
    create_junction(&inbox, &outside);

    assert!(Store::open(&session, Limits::default()).is_err());
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
}

#[cfg(windows)]
#[test]
fn windows_junction_blob_roots_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let blobs = store.dir().join("blobs");
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"keep").unwrap();
    create_junction(&blobs, &outside);

    assert!(store.write_blob_locked("body", b"{}").is_err());
    assert!(store.read_blob_locked("body", "").is_err());
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
}

#[cfg(windows)]
#[test]
fn windows_remove_dir_refuses_a_junction_inbox_root() {
    let temp = tempfile::tempdir().unwrap();
    let session = temp.path().join("session.jsonl");
    let inbox = session_inbox_dir(&session);
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"keep").unwrap();
    create_junction(&inbox, &outside);

    assert!(remove_dir(&session).is_err());
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
}

#[cfg(windows)]
#[test]
fn windows_remove_dir_does_not_follow_a_nested_junction() {
    let temp = tempfile::tempdir().unwrap();
    let session = temp.path().join("session.jsonl");
    let store = Store::open(&session, Limits::default()).unwrap();
    let blobs = store.dir().join("blobs");
    let outside = temp.path().join("outside");
    fs::create_dir(&blobs).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"keep").unwrap();
    fs::remove_dir(&blobs).unwrap();
    create_junction(&blobs, &outside);

    assert!(remove_dir(&session).is_err());
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
}

#[cfg(windows)]
fn create_junction(link: &Path, target: &Path) {
    let output = std::process::Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(output.status.success(), "mklink /J failed: {output:?}");
}

impl Store {
    fn write_blob_locked(&self, blob_name: &str, data: &[u8]) -> super::Result<()> {
        let state = self.state.lock().unwrap();
        self.write_blob_at_locked(&state, blob_name, data)
    }

    fn read_blob_locked(
        &self,
        blob_name: &str,
        want_checksum: &str,
    ) -> super::Result<super::PromptEnvelope> {
        let state = self.state.lock().unwrap();
        self.read_blob_at_locked(&state, blob_name, want_checksum)
    }
}

fn read_regular_file(path: &Path, max_bytes: u64) -> super::Result<Vec<u8>> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path.file_name().ok_or_else(|| Error::NotRegularFile {
        path: path.to_path_buf(),
    })?;
    let dir = Dir::open_ambient_dir(parent, ambient_authority()).unwrap();
    read_regular_file_in(&dir, Path::new(name), path, max_bytes)
}
