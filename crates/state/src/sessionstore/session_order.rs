//! Listing order for the sessions of one folder, plus the remembered
//! listing that keeps a warm folder from being re-read.
//!
//! Parity target: `reference/.../internal/state/sessionstore/session_order.go`
//! and `listindex.go` (docs/contracts/session-format.md).

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use reasonix_contract::gojson::{self, escape_html, is_zero_bool, is_zero_i64};
use reasonix_core::fileutil::{atomic_write_file, mkdir_all_mode};
use serde::{Deserialize, Serialize};

use super::branch::load_branch_meta;
use super::eventlog::record::{CreatedAt, deserialize_created_at};
use crate::store::naming::{
    is_session_transcript_name, session_cleanup_pending, session_event_log, session_meta,
};

/// Go listindex.go `listIndexName`: the remembered listing of one
/// session folder. It lives in the folder, beside the sessions it
/// describes, and is never a session itself.
const LIST_INDEX_NAME: &str = ".list-index.json";

/// Go listindex.go `listIndexFormat`: changes whenever
/// `SessionOrderInfo` or the way a row is read from the files
/// changes, so an index written by another build is not trusted.
const LIST_INDEX_FORMAT: i64 = 3;

/// Go listindex.go `listIndexMin`: the folder size from which an
/// index is kept. A small folder reads in milliseconds and is not worth
/// a file of its own.
const LIST_INDEX_MIN: usize = 100;

/// Go session_order.go `orderReaders`: the size of the reader pool
/// for a cold listing.
const ORDER_READERS: usize = 8;

/// Go session_order.go `SessionOrderInfo`: one row of the listing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionOrderInfo {
    #[serde(default, rename = "Path")]
    pub path: String,
    #[serde(
        default,
        rename = "CreatedAt",
        deserialize_with = "deserialize_created_at"
    )]
    pub created_at: CreatedAt,
    #[serde(
        default,
        rename = "LastActivityAt",
        deserialize_with = "deserialize_created_at"
    )]
    pub last_activity_at: CreatedAt,
    #[serde(
        default,
        rename = "ModTime",
        deserialize_with = "deserialize_created_at"
    )]
    pub mod_time: CreatedAt,
    #[serde(default, rename = "Scope")]
    pub scope: String,
    #[serde(default, rename = "WorkspaceRoot")]
    pub workspace_root: String,
    #[serde(default, rename = "TopicID")]
    pub topic_id: String,
    #[serde(default, rename = "TopicTitle")]
    pub topic_title: String,
    #[serde(default, rename = "CustomTitle")]
    pub custom_title: String,
    #[serde(default, rename = "Archived")]
    pub archived: bool,
    #[serde(default, rename = "Pinned")]
    pub pinned: bool,
    #[serde(default, rename = "ManualRank", serialize_with = "serialize_go_f64")]
    pub manual_rank: f64,
    #[serde(default, rename = "Unread")]
    pub unread: bool,
    #[serde(
        default,
        rename = "ViewedAt",
        deserialize_with = "deserialize_created_at"
    )]
    pub viewed_at: CreatedAt,
    #[serde(default, rename = "Recovered")]
    pub recovered: bool,
    #[serde(default, rename = "RecoveryReason")]
    pub recovery_reason: String,
    #[serde(default, rename = "RecoveryDigest")]
    pub recovery_digest: String,
    #[serde(default, rename = "ParentID")]
    pub parent_id: String,
    #[serde(default, rename = "RecoveryRootID")]
    pub recovery_root_id: String,
    #[serde(default, rename = "Turns")]
    pub turns: i64,
    #[serde(default, rename = "Preview")]
    pub preview: String,
    #[serde(default, rename = "SchemaVersion")]
    pub schema_version: i64,
    #[serde(default, rename = "Revision")]
    pub revision: i64,
    #[serde(default, rename = "ContentDigest")]
    pub content_digest: String,
}

/// Serializes an `f64` the way Go `encoding/json` renders one
/// in the rank domain: shortest decimal digits with no trailing `.0`,
/// so `0` is written where serde would write `0.0`. Non-finite
/// values never occur for validated ranks and fall back to the default
/// renderer.
fn serialize_go_f64<S: serde::Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 9.0e15 {
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f64(*value)
    }
}

/// Go listindex.go `fileStamp`: what one directory listing says
/// about a file, with no open: a size and a modification time. A file
/// that is absent has the zero stamp.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
struct FileStamp {
    #[serde(default, rename = "s", skip_serializing_if = "is_zero_i64")]
    size: i64,
    #[serde(default, rename = "m", skip_serializing_if = "is_zero_i64")]
    mod_nanos: i64,
    #[serde(default, rename = "h", skip_serializing_if = "is_zero_bool")]
    here: bool,
}

/// Go listindex.go `listPrint`: every file a session's listing row
/// is built from. The row is the same exactly while none of them
/// changed, which is how a remembered row is told from a stale one
/// without reading any of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ListPrint {
    #[serde(default, rename = "t")]
    transcript: FileStamp,
    #[serde(default, rename = "m")]
    meta: FileStamp,
    #[serde(default, rename = "e")]
    events: FileStamp,
}

/// Go listindex.go `listIndexEntry`: one remembered row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct ListIndexEntry {
    #[serde(default, rename = "p")]
    print: ListPrint,
    #[serde(default, rename = "x", skip_serializing_if = "is_zero_bool")]
    skip: bool,
    #[serde(default, rename = "i")]
    info: SessionOrderInfo,
}

/// Go listindex.go `listIndex`: the remembered listing of a folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ListIndex {
    #[serde(default)]
    format: i64,
    #[serde(default)]
    entries: BTreeMap<String, ListIndexEntry>,
}

/// Go listindex.go `stampSet`: answers file stamps for the files of
/// one folder from the listing that was already read.
struct StampSet {
    by_name: HashMap<String, FileStamp>,
}

impl StampSet {
    fn new(entries: &[fs::DirEntry]) -> Self {
        let mut by_name = HashMap::with_capacity(entries.len());
        for entry in entries {
            by_name.insert(
                entry.file_name().to_string_lossy().into_owned(),
                stamp_of_entry(entry),
            );
        }
        Self { by_name }
    }

    fn stamp(&self, name: &str) -> FileStamp {
        self.by_name.get(name).copied().unwrap_or_default()
    }

    /// Go `stampSet.of`: the print of a session's files, and
    /// `false` for a session no list shows — one whose removal is
    /// pending.
    fn of(&self, name: &str, full: &Path) -> (ListPrint, bool) {
        let pending = session_cleanup_pending(full);
        if let Some(pending_name) = pending.file_name() {
            let pending_name = pending_name.to_string_lossy().into_owned();
            if self.by_name.contains_key(&pending_name) {
                return (ListPrint::default(), false);
            }
        }
        (
            ListPrint {
                transcript: self.stamp(name),
                meta: self.stamp(&base_name(&session_meta(full))),
                events: self.stamp(&base_name(&session_event_log(full))),
            },
            true,
        )
    }
}

/// Go `stampSet.stamp` for one directory entry.
fn stamp_of_entry(entry: &fs::DirEntry) -> FileStamp {
    let Ok(file_type) = entry.file_type() else {
        return FileStamp::default();
    };
    if file_type.is_dir() {
        return FileStamp::default();
    }
    let Ok(info) = entry.metadata() else {
        return FileStamp::default();
    };
    FileStamp {
        size: info.len() as i64,
        mod_nanos: unix_nanos(info.modified()),
        here: true,
    }
}

/// Go `filepath.Base` for paths that are always plain file names
/// here; an empty path stays empty.
fn base_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn unix_nanos(at: io::Result<SystemTime>) -> i64 {
    at.ok()
        .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_nanos() as i64)
        .unwrap_or(0)
}

/// Go session_content.go `SessionContentModTime`: when the session
/// transcript last changed on disk — the newer of the `.jsonl`
/// checkpoint and the event log. The checkpoint alone goes stale between
/// checkpoints, so recency ordering must use this.
pub(crate) fn session_content_mod_time(session_path: &Path) -> Option<CreatedAt> {
    let mut mod_time: Option<SystemTime> = None;
    if let Ok(info) = fs::metadata(session_path)
        && !info.is_dir()
    {
        mod_time = info.modified().ok();
    }
    let log_path = session_event_log(session_path);
    if !log_path.as_os_str().is_empty()
        && let Ok(info) = fs::metadata(&log_path)
        && !info.is_dir()
        && let Ok(log_mod) = info.modified()
        && mod_time.as_ref().is_none_or(|known| log_mod > *known)
    {
        mod_time = Some(log_mod);
    }
    mod_time.map(system_time_to_created_at)
}

/// Converts a file timestamp into a `CreatedAt`.
fn system_time_to_created_at(at: SystemTime) -> CreatedAt {
    let utc = chrono::DateTime::<chrono::Utc>::from(at);
    let fixed = chrono::FixedOffset::east_opt(0).expect("UTC offset is representable");
    CreatedAt::from_datetime(utc.with_timezone(&fixed))
}

/// Go session_order.go `orderInfoOf`: reads one listing row from a
/// session's sidecars. The boolean is Go's skip: a row no list shows.
fn order_info_of(full: &Path) -> (SessionOrderInfo, bool) {
    let Ok(info) = fs::metadata(full) else {
        return (SessionOrderInfo::default(), true);
    };
    let mod_at = match info.modified() {
        Ok(at) => system_time_to_created_at(at),
        Err(_) => CreatedAt::zero(),
    };
    let content_mod = session_content_mod_time(full).unwrap_or(mod_at);
    let mut row = SessionOrderInfo {
        path: full.to_string_lossy().into_owned(),
        created_at: mod_at,
        last_activity_at: content_mod,
        mod_time: content_mod,
        scope: "global".to_string(),
        ..SessionOrderInfo::default()
    };
    match load_branch_meta(full) {
        Ok(Some(meta)) => {
            if meta.superseded {
                return (SessionOrderInfo::default(), true);
            }
            if !meta.created_at.is_zero() {
                row.created_at = meta.created_at;
            }
            if !meta.updated_at.is_zero() {
                row.last_activity_at = meta.updated_at;
                row.mod_time = meta.updated_at;
            }
            row.scope = default_scope(&meta.scope);
            row.workspace_root = meta.workspace_root;
            row.topic_id = meta.topic_id;
            row.topic_title = meta.topic_title;
            row.custom_title = meta.custom_title;
            row.archived = meta.archived;
            row.pinned = meta.pinned;
            row.manual_rank = meta.manual_rank;
            row.unread = meta.finished_at.get() > meta.viewed_at.get();
            row.viewed_at = meta.viewed_at;
            row.recovered = meta.recovered;
            row.recovery_reason = meta.recovery_reason;
            row.recovery_digest = meta.recovery_digest;
            row.parent_id = meta.parent_id;
            row.recovery_root_id = meta.recovery_root_id;
            row.turns = meta.turns;
            row.preview = meta.preview;
            row.schema_version = meta.schema_version;
            row.revision = meta.revision;
            row.content_digest = meta.content_digest;
            (row, false)
        }
        _ => (row, false),
    }
}

/// Go branch.go `DefaultScope`.
fn default_scope(scope: &str) -> String {
    if scope == "project" {
        "project".to_string()
    } else {
        "global".to_string()
    }
}

/// Go session_order.go `orderMiss`: one unread row of a listing.
struct OrderMiss {
    name: String,
    full: PathBuf,
    print: ListPrint,
    info: SessionOrderInfo,
    skip: bool,
}

fn read_one(miss: &mut OrderMiss) {
    let (info, skip) = order_info_of(&miss.full);
    miss.info = info;
    miss.skip = skip;
}

/// Go session_order.go `readOrderMisses`: reads the unread rows, in
/// parallel through a bounded pool of readers once there are enough of
/// them.
fn read_order_misses(misses: &mut [OrderMiss]) {
    if misses.len() < 2 * ORDER_READERS {
        for miss in misses.iter_mut() {
            read_one(miss);
        }
        return;
    }
    let chunk_size = misses.len().div_ceil(ORDER_READERS);
    std::thread::scope(|scope| {
        for chunk in misses.chunks_mut(chunk_size) {
            scope.spawn(move || {
                for miss in chunk.iter_mut() {
                    read_one(miss);
                }
            });
        }
    });
}

/// Go session_order.go `ListSessionOrder`: the sessions of one
/// folder, most recently active first, with the per-folder index that
/// keeps a warm listing from re-reading session sidecars.
pub fn list_session_order(dir: &Path) -> io::Result<Vec<SessionOrderInfo>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let entries: Vec<fs::DirEntry> = entries.flatten().collect();
    let stamps = StampSet::new(&entries);
    let was = load_list_index(dir);
    let mut next: BTreeMap<String, ListIndexEntry> = BTreeMap::new();
    let mut out: Vec<SessionOrderInfo> = Vec::new();
    let mut reused = 0usize;
    let mut misses: Vec<OrderMiss> = Vec::new();
    for entry in &entries {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() || !is_session_transcript_name(entry.file_name().as_os_str()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let full = dir.join(&name);
        let (print, ok) = stamps.of(&name, &full);
        if !ok {
            continue;
        }
        if let Some(remembered) = was.entries.get(&name)
            && remembered.print == print
        {
            reused += 1;
            next.insert(name.clone(), remembered.clone());
            if !remembered.skip {
                let mut info = remembered.info.clone();
                info.path = full.to_string_lossy().into_owned();
                out.push(info);
            }
            continue;
        }
        misses.push(OrderMiss {
            name,
            full,
            print,
            info: SessionOrderInfo::default(),
            skip: false,
        });
    }
    read_order_misses(&mut misses);
    for miss in &misses {
        next.insert(
            miss.name.clone(),
            ListIndexEntry {
                print: miss.print,
                skip: miss.skip,
                info: miss.info.clone(),
            },
        );
        if !miss.skip {
            out.push(miss.info.clone());
        }
    }
    save_list_index(dir, &was, &next, reused);
    out.sort_by(|a, b| {
        if a.last_activity_at.get() == b.last_activity_at.get() {
            return a.path.as_bytes().cmp(b.path.as_bytes());
        }
        b.last_activity_at.get().cmp(&a.last_activity_at.get())
    });
    Ok(out)
}

/// Go listindex.go `loadListIndex`: an unreadable or foreign index
/// reads as empty.
fn load_list_index(dir: &Path) -> ListIndex {
    let Ok(bytes) = fs::read(dir.join(LIST_INDEX_NAME)) else {
        return ListIndex::default();
    };
    let Ok(index) = serde_json::from_slice::<ListIndex>(&bytes) else {
        return ListIndex::default();
    };
    if index.format != LIST_INDEX_FORMAT {
        return ListIndex::default();
    }
    index
}

/// Go listindex.go `saveListIndex`: remembers this listing when it
/// differs from the last one and the folder is big enough to matter. It
/// is best effort: a folder that cannot be written just reads in full
/// next time.
fn save_list_index(
    dir: &Path,
    was: &ListIndex,
    now: &BTreeMap<String, ListIndexEntry>,
    reused: usize,
) {
    if now.len() < LIST_INDEX_MIN || (reused == now.len() && was.entries.len() == now.len()) {
        return;
    }
    let index = ListIndex {
        format: LIST_INDEX_FORMAT,
        entries: now.clone(),
    };
    let Ok(bytes) = gojson::marshal(&index) else {
        return;
    };
    static LIST_INDEX_MU: Mutex<()> = Mutex::new(());
    let _guard = LIST_INDEX_MU.lock().unwrap_or_else(|e| e.into_inner());
    let _ = atomic_write_file(&dir.join(LIST_INDEX_NAME), bytes.as_bytes(), 0o644);
}

/// Go save.go `CleanupPendingMeta`: the body of the delayed-removal
/// marker written beside a session.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CleanupPendingMeta {
    #[serde(default, rename = "operation")]
    pub operation: String,
    #[serde(default, rename = "createdAt")]
    pub created_at: i64,
}

/// Go save.go `MarkCleanupPending`: publishes the delayed-removal
/// marker atomically, so a crash cannot leave a malformed file behind.
pub fn mark_cleanup_pending(session_path: &str, operation: &str) -> io::Result<()> {
    let path = session_cleanup_pending(Path::new(session_path));
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        mkdir_all_mode(parent, 0o755)?;
    }
    let meta = CleanupPendingMeta {
        operation: operation.trim().to_string(),
        created_at: unix_millis_now(),
    };
    let json = serde_json::to_string_pretty(&meta).map_err(io::Error::other)?;
    let json = escape_html(&json);
    atomic_write_file(&path, json.as_bytes(), 0o644)
}

pub(crate) fn unix_millis_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

    use crate::sessionstore::branch::{BRANCH_META_READ_HOOK, BranchMeta, save_branch_meta};
    use crate::sessionstore::eventlog::test_support::tempdir;

    /// The folder whose sessions the installed hook observes; every other
    /// folder stays invisible to it, so tests that run in parallel do not
    /// pollute each other's counts.
    static HOOK_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
    static HOOK_COUNT: AtomicU64 = AtomicU64::new(0);
    static IN_FLIGHT: AtomicI32 = AtomicI32::new(0);
    static PEAK: AtomicI32 = AtomicI32::new(0);
    static SLEEP_HOOK: AtomicBool = AtomicBool::new(false);
    /// Serializes the hook-installing tests: the hook is process-global
    /// while installed.
    static HOOK_MU: Mutex<()> = Mutex::new(());

    fn branch_meta_read_hook(path: &Path) {
        let dir = HOOK_DIR.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(dir) = dir else {
            return;
        };
        if !path.starts_with(&dir) {
            return;
        }
        if SLEEP_HOOK.load(Ordering::SeqCst) {
            let in_flight = IN_FLIGHT.fetch_add(1, Ordering::SeqCst) + 1;
            PEAK.fetch_max(in_flight, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(2));
            IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        } else {
            HOOK_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn install_hook(dir: &Path, sleep: bool) {
        *HOOK_DIR.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir.to_path_buf());
        HOOK_COUNT.store(0, Ordering::SeqCst);
        IN_FLIGHT.store(0, Ordering::SeqCst);
        PEAK.store(0, Ordering::SeqCst);
        SLEEP_HOOK.store(sleep, Ordering::SeqCst);
        *BRANCH_META_READ_HOOK
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(branch_meta_read_hook);
    }

    fn clear_hook() {
        *BRANCH_META_READ_HOOK
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn hook_reads() -> u64 {
        HOOK_COUNT.load(Ordering::SeqCst)
    }

    fn write_sessions(dir: &Path, n: usize) {
        let base =
            chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z").expect("fixed base time");
        for i in 0..n {
            let name = format!("20261001-{i:06}-deepseek-flash.jsonl");
            let path = dir.join(&name);
            fs::write(&path, b"{\"role\":\"user\",\"content\":\"hello\"}\n")
                .expect("write transcript");
            let at = CreatedAt::from_datetime(base + chrono::Duration::minutes(i as i64));
            save_branch_meta(
                &path,
                BranchMeta {
                    id: name.clone(),
                    created_at: at,
                    updated_at: at,
                    turns: 3,
                    preview: format!("hello {name}"),
                    ..BranchMeta::default()
                },
            )
            .expect("save branch meta");
            let events = session_event_log(&path);
            fs::write(events, b"{}\n").expect("write event log");
        }
    }

    fn set_mtime(path: &Path, at: SystemTime) {
        let file = fs::File::options()
            .write(true)
            .open(path)
            .expect("open for set_modified");
        file.set_modified(at).expect("set modification time");
    }

    #[test]
    fn a_warm_folder_listing_does_not_re_read_session_sidecars() {
        let _hook_mu = HOOK_MU.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir();
        write_sessions(dir.path(), 300);
        install_hook(dir.path(), false);
        let first = list_session_order(dir.path()).expect("first listing");
        assert_eq!(hook_reads(), 300);
        let second = list_session_order(dir.path()).expect("second listing");
        assert_eq!(hook_reads(), 300);
        clear_hook();
        assert_eq!(first.len(), 300);
        assert_eq!(second.len(), 300);
        assert_eq!(first, second);
    }

    #[test]
    fn a_folder_with_enough_sessions_keeps_an_index() {
        let _hook_mu = HOOK_MU.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir();
        write_sessions(dir.path(), 150);
        assert!(!dir.path().join(LIST_INDEX_NAME).exists());
        install_hook(dir.path(), false);
        let first = list_session_order(dir.path()).expect("first listing");
        assert!(dir.path().join(LIST_INDEX_NAME).exists());
        let second = list_session_order(dir.path()).expect("second listing");
        assert_eq!(hook_reads(), 150);
        clear_hook();
        assert_eq!(first.len(), 150);
        assert_eq!(second.len(), 150);
        assert_eq!(first, second);
    }

    #[test]
    fn a_rewritten_meta_is_re_read_once() {
        let _hook_mu = HOOK_MU.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir();
        write_sessions(dir.path(), 200);
        install_hook(dir.path(), false);
        let first = list_session_order(dir.path()).expect("first listing");
        assert_eq!(first.len(), 200);
        let path = dir.path().join("20261001-000007-deepseek-flash.jsonl");
        let mut meta = load_branch_meta(&path)
            .expect("load branch meta")
            .expect("fixture meta present");
        meta.custom_title = "renamed".to_string();
        save_branch_meta(&path, meta).expect("rewrite branch meta");
        set_mtime(
            &session_meta(&path),
            SystemTime::now() + std::time::Duration::from_secs(3600),
        );
        let before = hook_reads();
        let second = list_session_order(dir.path()).expect("second listing");
        assert_eq!(hook_reads() - before, 1);
        clear_hook();
        assert_eq!(second.len(), 200);
        let row = second
            .iter()
            .find(|row| row.path.ends_with("20261001-000007-deepseek-flash.jsonl"))
            .expect("renamed session row");
        assert_eq!(row.custom_title, "renamed");
    }

    #[test]
    fn a_changed_folder_reads_only_the_new_session() {
        let _hook_mu = HOOK_MU.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir();
        write_sessions(dir.path(), 120);
        install_hook(dir.path(), false);
        let first = list_session_order(dir.path()).expect("first listing");
        assert_eq!(first.len(), 120);
        fs::remove_file(dir.path().join("20261001-000003-deepseek-flash.jsonl"))
            .expect("remove session transcript");
        fs::write(dir.path().join("20261002-000000-new.jsonl"), b"{}\n")
            .expect("write new session");
        let before = hook_reads();
        let second = list_session_order(dir.path()).expect("second listing");
        assert_eq!(hook_reads() - before, 1);
        clear_hook();
        assert_eq!(second.len(), 120);
        assert!(
            second
                .iter()
                .any(|row| row.path.ends_with("20261002-000000-new.jsonl"))
        );
        assert!(
            !second
                .iter()
                .any(|row| row.path.ends_with("20261001-000003-deepseek-flash.jsonl"))
        );
    }

    #[test]
    fn a_pending_removal_hides_a_session_without_reading_it() {
        let _hook_mu = HOOK_MU.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir();
        write_sessions(dir.path(), 120);
        install_hook(dir.path(), false);
        let first = list_session_order(dir.path()).expect("first listing");
        assert_eq!(first.len(), 120);
        let before = hook_reads();
        let hidden = dir.path().join("20261001-000009-deepseek-flash.jsonl");
        mark_cleanup_pending(hidden.to_string_lossy().as_ref(), "test")
            .expect("mark cleanup pending");
        let second = list_session_order(dir.path()).expect("second listing");
        assert_eq!(hook_reads(), before);
        clear_hook();
        assert_eq!(second.len(), 119);
        assert!(
            !second
                .iter()
                .any(|row| row.path.ends_with("20261001-000009-deepseek-flash.jsonl"))
        );
    }

    #[test]
    fn an_unusable_index_still_lists_every_session() {
        let _hook_mu = HOOK_MU.lock().unwrap_or_else(|e| e.into_inner());
        for body in ["not json", "{\"format\":999,\"entries\":{}}", ""] {
            let dir = tempdir();
            write_sessions(dir.path(), 120);
            fs::write(dir.path().join(LIST_INDEX_NAME), body).expect("write unusable index");
            install_hook(dir.path(), false);
            let rows = list_session_order(dir.path()).expect("listing with unusable index");
            assert_eq!(hook_reads(), 120);
            let again = list_session_order(dir.path()).expect("second listing");
            clear_hook();
            assert_eq!(rows.len(), 120);
            assert_eq!(rows, again);
        }
    }

    #[test]
    fn a_small_folder_is_not_indexed_and_a_big_one_is() {
        let small = tempdir();
        write_sessions(small.path(), 10);
        let rows = list_session_order(small.path()).expect("small listing");
        assert_eq!(rows.len(), 10);
        assert!(!small.path().join(LIST_INDEX_NAME).exists());

        let big = tempdir();
        write_sessions(big.path(), 110);
        let first = list_session_order(big.path()).expect("first big listing");
        let second = list_session_order(big.path()).expect("second big listing");
        assert_eq!(first.len(), 110);
        assert_eq!(second.len(), 110);
        assert_eq!(first, second);
    }

    #[test]
    fn concurrent_listings_agree() {
        let dir = tempdir();
        write_sessions(dir.path(), 150);
        let expected = list_session_order(dir.path()).expect("seed listing");
        let listings = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| list_session_order(dir.path()).expect("concurrent listing"))
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("listing thread"))
                .collect::<Vec<_>>()
        });
        for rows in &listings {
            assert_eq!(rows.len(), 150);
            assert_eq!(rows, &expected);
        }
    }

    #[test]
    fn a_session_order_info_serializes_to_the_row_shape() {
        let json = gojson::marshal(&SessionOrderInfo::default()).expect("marshal default row");
        assert_eq!(
            json,
            "{\"Path\":\"\",\"CreatedAt\":\"0001-01-01T00:00:00Z\",\"LastActivityAt\":\"0001-01-01T00:00:00Z\",\"ModTime\":\"0001-01-01T00:00:00Z\",\"Scope\":\"\",\"WorkspaceRoot\":\"\",\"TopicID\":\"\",\"TopicTitle\":\"\",\"CustomTitle\":\"\",\"Archived\":false,\"Pinned\":false,\"ManualRank\":0,\"Unread\":false,\"ViewedAt\":\"0001-01-01T00:00:00Z\",\"Recovered\":false,\"RecoveryReason\":\"\",\"RecoveryDigest\":\"\",\"ParentID\":\"\",\"RecoveryRootID\":\"\",\"Turns\":0,\"Preview\":\"\",\"SchemaVersion\":0,\"Revision\":0,\"ContentDigest\":\"\"}"
        );
    }

    #[test]
    fn a_cold_folder_is_read_by_a_bounded_pool_and_lists_the_same() {
        let _hook_mu = HOOK_MU.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir();
        write_sessions(dir.path(), 200);
        install_hook(dir.path(), true);
        let cold = list_session_order(dir.path()).expect("cold listing");
        SLEEP_HOOK.store(false, Ordering::SeqCst);
        clear_hook();
        let peak = PEAK.load(Ordering::SeqCst) as usize;
        assert!(
            (2..=ORDER_READERS).contains(&peak),
            "peak in-flight sidecar reads was {peak}"
        );
        let warm = list_session_order(dir.path()).expect("warm listing");
        assert_eq!(cold.len(), 200);
        assert_eq!(warm.len(), 200);
        assert_eq!(cold, warm);
    }
}
