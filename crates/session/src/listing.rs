//! Session listing read model: Go `sessionstore.ListNativeSessions`
//! (docs/contracts/session-format.md §7.14) plus the row shaping of serve
//! `sessions_list.go` — the `GET /sessions` body (http.md §7).
//!
//! Tolerance is the reference's (OPT-006): an unreadable directory answers
//! an empty list, never an error, and a row the sidecars cannot describe
//! is skipped, never guessed.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use reasonix_state::sessionstore::recovery_gc;
use reasonix_state::sessionstore::session::{canonical_session_path, is_visible_session};
use reasonix_state::sessionstore::session_order::{SessionOrderInfo, list_session_order};
use reasonix_state::store::naming::{is_subagent_transcript_name, session_event_log};

use crate::model::{SessionRow, Timestamp};
use crate::title::{TitleCache, session_title};

/// Go `branch.go` `branchMetaCountsInitialVersion` / `BranchMetaCountsVersion`.
const BRANCH_META_COUNTS_INITIAL_VERSION: i64 = 1;
const BRANCH_META_COUNTS_VERSION: i64 = 2;

/// One listed session row: the sidecar/mtime record plus whether its
/// counts were recorded from content (Go `SessionInfo.CountsKnown`).
#[derive(Debug, Clone)]
pub struct ListedSession {
    pub info: SessionOrderInfo,
    pub counts_known: bool,
}

/// Go `ListNativeSessions`: metadata rows for every visible session under
/// `dir`, most-recently-active first. A missing directory is an empty
/// list; an unreadable one is the caller's error to tolerate.
pub fn list_native_sessions(dir: &Path) -> io::Result<Vec<ListedSession>> {
    let ordered = list_session_order(dir)?;
    let mut out = Vec::with_capacity(ordered.len());
    for info in ordered {
        if counts_need_refresh(info.schema_version, info.turns) {
            if !artifacts_have_content(Path::new(&info.path)) {
                continue;
            }
            // A session whose counts are not yet recorded shows the
            // indexing fallback rather than an empty preview.
            let mut info = info;
            if info.preview.trim().is_empty() {
                info.preview = format!("History is being indexed — {}", file_stem(&info.path));
            }
            out.push(ListedSession {
                info,
                counts_known: false,
            });
            continue;
        }
        if info.turns == 0 {
            // Never had user interaction — an empty conversation appears in
            // no list.
            continue;
        }
        out.push(ListedSession {
            info,
            counts_known: true,
        });
    }
    Ok(out)
}

/// Go `sessionListingCountsNeedRefresh`: sidecar counts recorded by an
/// older build cannot be trusted for the lists that hide empty sessions.
fn counts_need_refresh(schema_version: i64, turns: i64) -> bool {
    if schema_version < BRANCH_META_COUNTS_INITIAL_VERSION {
        return true;
    }
    turns == 0 && schema_version < BRANCH_META_COUNTS_VERSION
}

/// Go `sessionArtifactsHaveContent`: a transcript or event log with bytes.
fn artifacts_have_content(session_path: &Path) -> bool {
    if let Ok(info) = fs::metadata(session_path)
        && !info.is_dir()
        && info.len() > 0
    {
        return true;
    }
    event_log_size(session_path) > 0
}

fn event_log_size(session_path: &Path) -> u64 {
    let log = session_event_log(session_path);
    if log.as_os_str().is_empty() {
        return 0;
    }
    fs::metadata(&log)
        .map(|info| if info.is_dir() { 0 } else { info.len() })
        .unwrap_or(0)
}

/// Go `SessionContentModTime`: when the conversation last changed on disk —
/// the newer of the `.jsonl` checkpoint and the event log. The checkpoint
/// alone goes stale between checkpoints.
pub fn session_content_mod_time(session_path: &Path) -> Timestamp {
    let mut mod_time: Option<std::time::SystemTime> = None;
    if let Ok(info) = fs::metadata(session_path)
        && !info.is_dir()
    {
        mod_time = info.modified().ok();
    }
    let log = session_event_log(session_path);
    if !log.as_os_str().is_empty()
        && let Ok(info) = fs::metadata(&log)
        && !info.is_dir()
        && let Ok(log_mod) = info.modified()
        && mod_time.is_none_or(|known| log_mod > known)
    {
        mod_time = Some(log_mod);
    }
    mod_time
        .map(Timestamp::from_system_time)
        .unwrap_or_default()
}

/// Go serve `hiddenRecoveryCopy`: a conflict-recovery fork that preserves
/// nothing its parent lacks. A save-conflict loop mints one per save, and
/// listing them fills the sidebar with identical-looking copies. The
/// session in use is never hidden, and coverage is proven from content.
pub fn hidden_recovery_copy(info: &SessionOrderInfo, current: &str) -> bool {
    if !info.recovered {
        return false;
    }
    if canonical_session_path(&info.path) == current {
        return false;
    }
    let path = Path::new(&info.path);
    recovery_gc::recovery_branch_covered_by_parent(path, &go_dir(path))
}

/// Go `filepath.Dir` of a session path, "." for a bare name.
fn go_dir(path: &Path) -> PathBuf {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Go serve `compareSessions`: hand-placed rows first by the rank the
/// person gave them, then never-placed rows newest activity first, paths
/// tie-breaking byte-wise.
pub fn compare_sessions(a: &SessionOrderInfo, b: &SessionOrderInfo) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.manual_rank == 0.0, b.manual_rank == 0.0) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (false, false) if a.manual_rank != b.manual_rank => {
            return a
                .manual_rank
                .partial_cmp(&b.manual_rank)
                .unwrap_or(Ordering::Equal);
        }
        _ => {}
    }
    if a.manual_rank == 0.0 && b.manual_rank == 0.0 {
        let by_activity = b.last_activity_at.get().cmp(&a.last_activity_at.get());
        if by_activity != Ordering::Equal {
            return by_activity;
        }
    }
    a.path.as_bytes().cmp(b.path.as_bytes())
}

/// The `GET /sessions` body for one transcript directory
/// (serve `sessions_list.go`): subagent transcripts and hidden recovery
/// copies excluded, titles from sidecars, unreadable directory answering
/// `[]`. `current` is the canonical path of the session a pane is driving;
/// the hub read side has none and passes "".
pub fn session_rows(dir: &Path, current: &str, titles: &TitleCache) -> Vec<SessionRow> {
    let listed = match list_native_sessions(dir) {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(dir = %dir.display(), err = %err, "list sessions");
            return Vec::new();
        }
    };
    // One sidecar read per directory, one per listing (Go: the snapshot is
    // taken once, outside the row loop).
    let titles = titles.snapshot();
    let mut out = Vec::with_capacity(listed.len());
    for listed in listed {
        let info = &listed.info;
        let base = file_name(&info.path);
        // A subagent transcript only means anything through the session
        // that spawned it; only flat legacy names reach here.
        if is_subagent_transcript_name(OsStr::new(&base)) {
            continue;
        }
        if hidden_recovery_copy(info, current) {
            continue;
        }
        let modified = session_content_mod_time(Path::new(&info.path));
        let title = session_title(
            &titles,
            &base,
            &info.preview,
            modified.unix_nanos(),
            &info.custom_title,
        );
        out.push(SessionRow {
            name: base.strip_suffix(".jsonl").unwrap_or(&base).to_string(),
            path: info.path.clone(),
            title,
            turns: info.turns,
            current: !current.is_empty() && canonical_session_path(&info.path) == current,
            unread: info.unread,
            modified,
        });
    }
    out
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn file_stem(path: &str) -> String {
    file_name(path)
        .strip_suffix(".jsonl")
        .unwrap_or(path)
        .to_string()
}

/// Go `IsVisibleSession`, re-exported shape for callers composing their
/// own open-pane rows: cleanup-pending sessions appear in no list.
pub fn is_visible(path: &str) -> bool {
    is_visible_session(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reasonix_contract::provider::{Message, Role};
    use reasonix_state::sessionstore::branch::{BranchMeta, branch_id, save_branch_meta};
    use reasonix_state::sessionstore::eventlog::writer_id::digest_string;
    use reasonix_state::sessionstore::session::{digest_session_messages, load_session};

    fn user_message(content: &str) -> Message {
        Message {
            role: Role::USER,
            content: content.to_string(),
            ..Message::default()
        }
    }

    /// Writes a transcript whose checkpoint is the given messages, the way
    /// `writeSessionMessages` encodes them (one JSON message per line).
    fn write_transcript(dir: &Path, stem: &str, msgs: &[Message]) -> PathBuf {
        let path = dir.join(format!("{stem}.jsonl"));
        let mut body = String::new();
        for m in msgs {
            body.push_str(&serde_json::to_string(m).unwrap());
            body.push('\n');
        }
        fs::write(&path, body).unwrap();
        path
    }

    fn set_meta(path: &Path, apply: impl FnOnce(&mut BranchMeta)) {
        let mut meta = BranchMeta::default();
        apply(&mut meta);
        save_branch_meta(path, meta).unwrap();
    }

    fn loaded_digest(path: &Path) -> String {
        let session = load_session(&path.to_string_lossy()).unwrap();
        digest_string(&digest_session_messages(&session.snapshot()).unwrap())
    }

    /// A sidecar timestamp from its wire form (CreatedAt cannot be built
    /// directly outside the state crate).
    fn at(text: &str) -> reasonix_state::sessionstore::eventlog::record::CreatedAt {
        let wire = serde_json::to_string(text).unwrap();
        let mut de = serde_json::Deserializer::from_str(&wire);
        reasonix_state::sessionstore::eventlog::record::deserialize_created_at(&mut de).unwrap()
    }

    #[test]
    fn empty_and_missing_directories_list_empty() {
        let tmp = crate::test_support::tempdir();
        assert!(list_native_sessions(tmp.path()).unwrap().is_empty());
        assert!(
            list_native_sessions(&tmp.path().join("absent"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn sessions_without_counts_fall_back_to_the_indexing_preview() {
        let tmp = crate::test_support::tempdir();
        write_transcript(
            tmp.path(),
            "20260101-000000-000-model",
            &[user_message("find me")],
        );
        let rows = list_native_sessions(tmp.path()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].counts_known);
        assert_eq!(
            rows[0].info.preview,
            "History is being indexed — 20260101-000000-000-model"
        );
    }

    #[test]
    fn zero_byte_sessions_without_counts_are_skipped() {
        let tmp = crate::test_support::tempdir();
        fs::write(tmp.path().join("20260101-000000-000-model.jsonl"), "").unwrap();
        assert!(list_native_sessions(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn sessions_with_known_counts_and_no_turns_are_skipped() {
        let tmp = crate::test_support::tempdir();
        let path = write_transcript(
            tmp.path(),
            "20260101-000000-000-model",
            &[user_message("hi")],
        );
        set_meta(&path, |meta| {
            meta.schema_version = BRANCH_META_COUNTS_VERSION;
            meta.turns = 0;
        });
        assert!(list_native_sessions(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn counted_sessions_keep_their_sidecar_preview() {
        let tmp = crate::test_support::tempdir();
        let path = write_transcript(
            tmp.path(),
            "20260101-000000-000-model",
            &[user_message("hi")],
        );
        set_meta(&path, |meta| {
            meta.schema_version = BRANCH_META_COUNTS_VERSION;
            meta.turns = 3;
            meta.preview = "hi".into();
        });
        let rows = list_native_sessions(tmp.path()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].counts_known);
        assert_eq!(rows[0].info.turns, 3);
    }

    #[test]
    fn subagent_transcripts_never_list() {
        let tmp = crate::test_support::tempdir();
        write_transcript(tmp.path(), "subagent-abc123", &[user_message("hi")]);
        // The raw listing keeps them (GC and redaction still see them);
        // the /sessions row pass is what excludes them.
        assert_eq!(list_native_sessions(tmp.path()).unwrap().len(), 1);
        let rows = session_rows(tmp.path(), "", &TitleCache::new(tmp.path()));
        assert!(rows.is_empty());
    }

    #[test]
    fn covered_recovery_copies_are_hidden_but_live_ones_are_not() {
        let tmp = crate::test_support::tempdir();
        let msgs = vec![user_message("find me")];
        let parent = write_transcript(tmp.path(), "20260101-000000-000-model", &msgs);
        let copy = write_transcript(tmp.path(), "20260101-000000-001-model", &msgs);
        let digest = loaded_digest(&copy);
        set_meta(&copy, |meta| {
            meta.id = branch_id(&copy.to_string_lossy());
            meta.parent_id = branch_id(&parent.to_string_lossy());
            meta.recovered = true;
            meta.recovery_digest = digest.clone();
        });
        let rows = list_native_sessions(tmp.path()).unwrap();
        assert_eq!(rows.len(), 2, "the listing itself never hides");
        assert!(
            hidden_recovery_copy(&rows[0].info, ""),
            "covered copy hides"
        );
        // The copy in use is never hidden.
        let current = canonical_session_path(&copy.to_string_lossy());
        let rows = session_rows(tmp.path(), &current, &TitleCache::new(tmp.path()));
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn unrecovered_rows_are_never_hidden() {
        let tmp = crate::test_support::tempdir();
        let path = write_transcript(
            tmp.path(),
            "20260101-000000-000-model",
            &[user_message("hi")],
        );
        set_meta(&path, |meta| {
            meta.schema_version = BRANCH_META_COUNTS_VERSION;
            meta.turns = 1;
        });
        let rows = list_native_sessions(tmp.path()).unwrap();
        assert!(!hidden_recovery_copy(&rows[0].info, ""));
    }

    #[test]
    fn session_rows_match_the_http_shape() {
        let tmp = crate::test_support::tempdir();
        let path = write_transcript(
            tmp.path(),
            "20260101-000000-000-model",
            &[user_message("find me")],
        );
        set_meta(&path, |meta| {
            meta.schema_version = BRANCH_META_COUNTS_VERSION;
            meta.turns = 1;
            meta.preview = "find me".into();
            // Unread is derived, not stored: a turn finished after the
            // last look (Go: finished_at > viewed_at).
            meta.finished_at = at("2026-01-02T00:00:00Z");
        });
        let rows = session_rows(tmp.path(), "", &TitleCache::new(tmp.path()));
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.name, "20260101-000000-000-model");
        assert_eq!(row.path, path.to_string_lossy());
        assert!(row.unread);
        assert!(!row.current);
        // The title is the preview until a title machine fills the cache.
        assert_eq!(row.title, "find me");
        assert!(!row.modified.is_zero());
    }

    #[test]
    fn compare_sessions_orders_unplaced_rows_first_newest_activity() {
        let placed = SessionOrderInfo {
            manual_rank: 2048.0,
            ..SessionOrderInfo::default()
        };
        let newer = SessionOrderInfo {
            last_activity_at: at("2026-01-03T00:00:00Z"),
            ..SessionOrderInfo::default()
        };
        let older = SessionOrderInfo {
            last_activity_at: at("2026-01-02T00:00:00Z"),
            ..SessionOrderInfo::default()
        };
        // The ones never placed by hand come first (Go compareSessions).
        assert_eq!(
            compare_sessions(&placed, &newer),
            std::cmp::Ordering::Greater
        );
        // Among the unplaced, newest activity first.
        assert_eq!(compare_sessions(&newer, &older), std::cmp::Ordering::Less);
        assert_eq!(
            compare_sessions(&older, &newer),
            std::cmp::Ordering::Greater
        );
        assert_eq!(compare_sessions(&newer, &newer), std::cmp::Ordering::Equal);
    }
}
