//! Session listing helpers over the store's own metadata (Go:
//! `session_listing_helpers.go`, the listing half of `save.go`, and the
//! disk-side projections of `sessionpreview.go`).
//!
//! A listing row is built from file metadata and the branch sidecar, never
//! from transcript content. Only a row whose cached counts are missing or
//! untrustworthy is decoded, and a session that never gained a user turn is
//! listed nowhere.

use std::fs;
use std::io;
use std::path::Path;

use reasonix_contract::provider::Message;

use super::branch::{
    BRANCH_META_COUNTS_VERSION, BranchMeta, BranchMetaError, ensure_branch_meta_unlocked,
    load_branch_meta, lock_session_meta_path, save_branch_meta_after,
};
use super::eventlog::session_event_log_size;
use super::session::load_session_messages;
use super::session_order::{SessionOrderInfo, list_session_order};
use super::sessionpreview::session_preview_from_messages;
use crate::store::naming::session_event_log;

/// The prefix a listed session shows while its counts are still unknown
/// (Go: `"History is being indexed — "`).
pub const SESSION_LISTING_INDEXING_PREFIX: &str = "History is being indexed — ";

/// First sidecar schema that carries any counts (Go:
/// `branchMetaCountsInitialVersion`).
const BRANCH_META_COUNTS_INITIAL_VERSION: i64 = 1;

/// One listed session for the `--resume` picker: the ordering row plus the
/// preview, the turn count, and whether those counts came from content (Go:
/// `SessionInfo`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionInfo {
    /// Why a row is not native content (Go: `MetadataState`); empty for
    /// native sessions.
    pub metadata_state: String,
    /// Transcript path.
    pub path: String,
    /// Creation time from the sidecar or the file.
    pub created_at: crate::sessionstore::eventlog::record::CreatedAt,
    /// Last activity time.
    pub last_activity_at: crate::sessionstore::eventlog::record::CreatedAt,
    /// Compatibility alias of `last_activity_at` (Go: `ModTime`).
    pub mod_time: crate::sessionstore::eventlog::record::CreatedAt,
    /// First user turn, truncated for the listing.
    pub preview: String,
    /// User-turn count.
    pub turns: i64,
    /// Whether `preview`/`turns` were recorded from content.
    pub counts_known: bool,
    /// Listing scope (`global` or `project`).
    pub scope: String,
    /// Workspace root the session belongs to.
    pub workspace_root: String,
    /// Topic identity.
    pub topic_id: String,
    /// Topic title.
    pub topic_title: String,
    /// Operator-assigned title.
    pub custom_title: String,
    /// Archived out of the everyday list.
    pub archived: bool,
    /// Pinned against automatic archiving.
    pub pinned: bool,
    /// Manual rank; 0 means never placed.
    pub manual_rank: f64,
    /// Unread since the last view.
    pub unread: bool,
    /// Last view time.
    pub viewed_at: crate::sessionstore::eventlog::record::CreatedAt,
    /// Whether this is a recovery branch.
    pub recovered: bool,
    /// Why it was recovered.
    pub recovery_reason: String,
    /// Digest the recovery branched from.
    pub recovery_digest: String,
    /// Parent of a branch or recovery.
    pub parent_id: String,
    /// Conversation the recovery chain came from.
    pub recovery_root_id: String,
}

impl SessionInfo {
    /// Projects an ordering row (Go: `sessionInfoFromOrder`).
    pub fn from_order(
        session: &SessionOrderInfo,
        preview: String,
        turns: i64,
        counts_known: bool,
    ) -> SessionInfo {
        SessionInfo {
            metadata_state: String::new(),
            path: session.path.clone(),
            created_at: session.created_at,
            last_activity_at: session.last_activity_at,
            mod_time: session.mod_time,
            preview,
            turns,
            counts_known,
            scope: session.scope.clone(),
            workspace_root: session.workspace_root.clone(),
            topic_id: session.topic_id.clone(),
            topic_title: session.topic_title.clone(),
            custom_title: session.custom_title.clone(),
            archived: session.archived,
            pinned: session.pinned,
            manual_rank: session.manual_rank,
            unread: session.unread,
            viewed_at: session.viewed_at,
            recovered: session.recovered,
            recovery_reason: session.recovery_reason.clone(),
            recovery_digest: session.recovery_digest.clone(),
            parent_id: session.parent_id.clone(),
            recovery_root_id: session.recovery_root_id.clone(),
        }
    }
}

/// Whether a transcript or its event log holds bytes (Go:
/// `sessionArtifactsHaveContent`).
pub fn session_artifacts_have_content(session_path: &Path) -> bool {
    if let Ok(info) = fs::metadata(session_path)
        && !info.is_dir()
        && info.len() > 0
    {
        return true;
    }
    session_event_log_size(session_path) > 0
}

/// Whether the cached counts must be decoded again (Go:
/// `sessionListingCountsNeedRefresh`): a sidecar older than the first counts
/// schema, or a zero count a build older than v2 may have cached from a
/// failed decode.
pub fn session_listing_counts_need_refresh(schema_version: i64, turns: i64) -> bool {
    if schema_version < BRANCH_META_COUNTS_INITIAL_VERSION {
        return true;
    }
    turns == 0 && schema_version < BRANCH_META_COUNTS_VERSION
}

/// Backfills `preview`/`turns` under the meta lock, but only while the
/// sidecar still matches the listing row's compare-and-apply guards (Go:
/// `updateSessionListingCountsIfCurrent`). Returns the counts a caller should
/// publish, whether or not this call wrote them.
pub fn update_session_listing_counts_if_current(
    session: &SessionOrderInfo,
    preview: String,
    turns: i64,
) -> Result<(String, i64), BranchMetaError> {
    let _lock = lock_session_meta_path(Path::new(&session.path))?;
    let mut current = ensure_branch_meta_unlocked(Path::new(&session.path))?;
    let unchanged = current.schema_version == session.schema_version
        && current.turns == session.turns
        && current.preview == session.preview
        && current.revision == session.revision
        && current.content_digest == session.content_digest;
    if !unchanged || !session_listing_counts_need_refresh(current.schema_version, current.turns) {
        return Ok((current.preview, current.turns));
    }
    let persisted = current.clone();
    current.preview = preview.clone();
    current.turns = turns;
    current.schema_version = BRANCH_META_COUNTS_VERSION;
    save_branch_meta_after(
        Path::new(&session.path),
        &mut current,
        false,
        Some(&persisted),
    )?;
    Ok((preview, turns))
}

/// Decodes one session for its preview and turn count (Go:
/// `previewSessionWithError`).
pub fn preview_session_with_error(path: &Path) -> io::Result<(String, i64)> {
    let (msgs, _, _) =
        load_session_messages(path).map_err(|err| io::Error::other(err.to_string()))?;
    Ok(session_preview_from_messages(&msgs))
}

/// The preview and turn count the resume picker shows for one file (Go:
/// `SessionPreview`). Errors are swallowed: a malformed file shows an empty
/// preview.
pub fn session_preview(path: &Path) -> (String, i64) {
    preview_session_with_error(path).unwrap_or_default()
}

/// Every native session under `dir`, most recently active first (Go:
/// `ListNativeSessions`). A row whose counts are cached is listed without
/// decoding the transcript; a stale one is decoded unless it holds no
/// content, and a session with zero turns is listed nowhere.
pub fn list_native_sessions(dir: &Path) -> io::Result<Vec<SessionInfo>> {
    let ordered = list_session_order(dir)?;
    let mut out = Vec::with_capacity(ordered.len());
    for session in ordered {
        let (mut preview, turns) = (session.preview.clone(), session.turns);
        if session_listing_counts_need_refresh(session.schema_version, turns) {
            if !session_artifacts_have_content(Path::new(&session.path)) {
                continue;
            }
            if preview.trim().is_empty() {
                preview = format!(
                    "{SESSION_LISTING_INDEXING_PREFIX}{}",
                    file_base(&session.path)
                );
            }
            out.push(SessionInfo::from_order(&session, preview, turns, false));
            continue;
        }
        if turns == 0 {
            // Never had user interaction — an empty conversation appears in
            // no history panel and no resume picker.
            continue;
        }
        out.push(SessionInfo::from_order(&session, preview, turns, true));
    }
    Ok(out)
}

/// Every session a list should show (Go: `ListSessions`): native rows plus
/// unimported 1.x v4 conversations, ordered by latest activity.
pub fn list_sessions(dir: &Path) -> io::Result<Vec<SessionInfo>> {
    let native = list_native_sessions(dir)?;
    Ok(super::legacy_listings::with_v4_sessions(dir, native))
}

/// Where a conversation carried into a rebuilt controller keeps auto-saving
/// (Go: `ContinueSessionPath`): its existing file when it has one, so the
/// continued session stays one file instead of orphaning an identical
/// duplicate; a fresh path when it has none; `""` when persistence is off.
pub fn continue_session_path(prev_path: &str, dir: &str, model: &str) -> String {
    if !prev_path.is_empty() {
        return prev_path.to_string();
    }
    if dir.is_empty() {
        return String::new();
    }
    super::path::new_session_path(dir, model)
        .to_string_lossy()
        .into_owned()
}

/// The last 300 runes of every human-authored turn, in order (Go:
/// `UserMessagesTitleSource`): the source a title is derived from.
pub fn user_messages_title_source(path: &Path) -> io::Result<String> {
    let (msgs, _, _) =
        load_session_messages(path).map_err(|err| io::Error::other(err.to_string()))?;
    Ok(user_messages_title_source_from_messages(&msgs))
}

/// The in-memory half of [`user_messages_title_source`], so a caller holding
/// the transcript does not re-read the file.
pub fn user_messages_title_source_from_messages(msgs: &[Message]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for m in msgs {
        if m.role != reasonix_contract::provider::Role::USER || m.host_authored {
            continue;
        }
        let text = super::preview::user_message_text(m);
        if !super::preview::is_user_authored_turn(&text) {
            continue;
        }
        let text = super::preview::strip_paste_display_label(&text)
            .trim()
            .to_string();
        if !text.is_empty() {
            parts.push(text);
        }
    }
    let joined = parts.join("\n");
    let runes: Vec<char> = joined.chars().collect();
    if runes.len() > 300 {
        runes[runes.len() - 300..].iter().collect()
    } else {
        joined
    }
}

/// Go `filepath.Base` for a session path.
fn file_base(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// Reads a session's sidecar for callers that only need the listing fields
/// (Go: the `loadBranchMeta` half of the listing row).
pub fn load_listing_meta(session_path: &Path) -> Result<Option<BranchMeta>, BranchMetaError> {
    load_branch_meta(session_path)
}

/// The event log's on-disk size, exposed here so listing callers can test
/// "has content" without importing the event-log module (Go:
/// `sessionEventLogSize`).
pub fn listed_event_log_size(session_path: &Path) -> i64 {
    let log = session_event_log(session_path);
    if log.as_os_str().is_empty() {
        return 0;
    }
    session_event_log_size(session_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::branch::save_branch_meta;
    use crate::sessionstore::eventlog::test_support::tempdir;
    use crate::sessionstore::session::Session;
    use reasonix_contract::provider::Role;

    fn user(content: &str) -> Message {
        Message {
            role: Role::USER,
            content: content.to_string(),
            ..Message::default()
        }
    }

    fn assistant(content: &str) -> Message {
        Message {
            role: Role::ASSISTANT,
            content: content.to_string(),
            ..Message::default()
        }
    }

    fn saved_session(dir: &Path, name: &str, turns: &[&str]) -> std::path::PathBuf {
        let path = dir.join(name);
        let session = Session::new("sys");
        for turn in turns {
            session.add(user(turn));
            session.add(assistant("ok"));
        }
        session.save(&path.to_string_lossy()).expect("save");
        path
    }

    #[test]
    fn counts_refresh_rule_matches_the_sidecar_schemas() {
        assert!(session_listing_counts_need_refresh(0, 5));
        assert!(session_listing_counts_need_refresh(0, 0));
        assert!(session_listing_counts_need_refresh(1, 0));
        assert!(!session_listing_counts_need_refresh(1, 3));
        assert!(!session_listing_counts_need_refresh(2, 0));
    }

    #[test]
    fn artifacts_have_content_needs_bytes_in_the_transcript_or_log() {
        let dir = tempdir();
        let empty = dir.path().join("empty.jsonl");
        std::fs::write(&empty, b"").expect("write empty");
        assert!(!session_artifacts_have_content(&empty));

        let transcript = dir.path().join("t.jsonl");
        std::fs::write(&transcript, b"{}\n").expect("write transcript");
        assert!(session_artifacts_have_content(&transcript));

        let log_only = dir.path().join("log-only.jsonl");
        std::fs::write(session_event_log(&log_only), b"{}").expect("write log");
        assert!(session_artifacts_have_content(&log_only));
        assert_eq!(listed_event_log_size(&log_only), 2);
    }

    #[test]
    fn a_saved_session_is_listed_with_its_recorded_counts() {
        let dir = tempdir();
        let _path = saved_session(dir.path(), "one.jsonl", &["first question"]);
        let rows = list_native_sessions(dir.path()).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].preview, "first question");
        assert_eq!(rows[0].turns, 1);
        assert!(rows[0].counts_known);
        assert_eq!(rows[0].scope, "global");
    }

    #[test]
    fn a_session_with_no_user_turn_is_listed_nowhere() {
        let dir = tempdir();
        let empty = dir.path().join("empty.jsonl");
        let session = Session::new("sys");
        session.add(assistant("only a system prompt and an answer"));
        session.save(&empty.to_string_lossy()).expect("save");
        assert!(list_native_sessions(dir.path()).expect("list").is_empty());
    }

    #[test]
    fn a_stale_zero_count_is_revalidated_then_trusted() {
        let dir = tempdir();
        let path = saved_session(dir.path(), "stale.jsonl", &["a real turn"]);
        // v1 sidecars could cache a failed decode as zero turns.
        let mut meta = load_branch_meta(&path).expect("meta io").expect("meta");
        meta.schema_version = 1;
        meta.turns = 0;
        meta.preview = String::new();
        save_branch_meta(&path, meta).expect("rewrite meta");

        let rows = list_native_sessions(dir.path()).expect("list");
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].counts_known);
        assert_eq!(rows[0].preview, "History is being indexed — stale.jsonl");
        assert_eq!(rows[0].turns, 0);

        // The compare-and-apply backfill certifies the decoded counts.
        let row = list_session_order(dir.path()).expect("order").remove(0);
        let (preview, turns) =
            update_session_listing_counts_if_current(&row, "a real turn".into(), 1)
                .expect("backfill");
        assert_eq!((preview.as_str(), turns), ("a real turn", 1));
        let meta = load_branch_meta(&path).expect("meta io").expect("meta");
        assert_eq!(meta.schema_version, BRANCH_META_COUNTS_VERSION);
        assert_eq!(meta.turns, 1);
        assert_eq!(meta.preview, "a real turn");
    }

    #[test]
    fn a_concurrent_writer_wins_the_backfill() {
        let dir = tempdir();
        let path = saved_session(dir.path(), "race.jsonl", &["one"]);
        let mut stale = SessionOrderInfo {
            path: path.to_string_lossy().into_owned(),
            schema_version: 1,
            ..SessionOrderInfo::default()
        };
        // The sidecar has moved on (revision 1 plus a fresh digest) since the
        // listing row was read, so the row must not overwrite it.
        stale.revision = 99;
        let (preview, turns) =
            update_session_listing_counts_if_current(&stale, "stale".into(), 7).expect("declined");
        assert_eq!(turns, 1);
        assert_ne!(preview, "stale");
        let meta = load_branch_meta(&path).expect("meta io").expect("meta");
        assert_eq!(meta.turns, 1);
    }

    #[test]
    fn a_cached_row_is_listed_without_decoding_the_transcript() {
        let dir = tempdir();
        let path = saved_session(dir.path(), "cached.jsonl", &["first", "second"]);
        // Poison the transcript: a decode would fail, but cached counts mean
        // the listing never reads it.
        std::fs::write(&path, b"not json at all\n").expect("poison transcript");
        let rows = list_native_sessions(dir.path()).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].turns, 2);
        assert!(rows[0].counts_known);
        assert_eq!(rows[0].preview, "first");
    }

    #[test]
    fn indexing_fallback_names_the_file() {
        let dir = tempdir();
        let path = dir.path().join("never-indexed.jsonl");
        std::fs::write(&path, b"{\"role\":\"user\",\"content\":\"x\"}\n").expect("write");
        // A sidecar with no counts schema at all: the row shows the indexing
        // fallback while its content exists.
        let meta = BranchMeta {
            schema_version: 0,
            ..BranchMeta::default()
        };
        save_branch_meta(&path, meta).expect("meta");
        let rows = list_native_sessions(dir.path()).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].preview,
            "History is being indexed — never-indexed.jsonl"
        );
        assert!(!rows[0].counts_known);
    }

    #[test]
    fn recorded_counts_win_over_the_transcript() {
        let dir = tempdir();
        let path = saved_session(dir.path(), "planted.jsonl", &["real content in the file"]);
        // Deliberately disagree with the file: if the listing returns these,
        // it read the sidecar rather than decoding the transcript.
        let mut meta = load_branch_meta(&path).expect("meta io").expect("meta");
        meta.turns = 3;
        meta.preview = "cached preview line".to_string();
        save_branch_meta(&path, meta).expect("plant");

        let rows = list_native_sessions(dir.path()).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].turns, 3);
        assert_eq!(rows[0].preview, "cached preview line");
        assert!(rows[0].counts_known);
    }

    #[test]
    fn a_counts_authoritative_empty_session_is_skipped_without_decoding() {
        let dir = tempdir();
        let path = saved_session(
            dir.path(),
            "claims-empty.jsonl",
            &["content the meta denies"],
        );
        let mut meta = load_branch_meta(&path).expect("meta io").expect("meta");
        meta.turns = 0;
        meta.schema_version = BRANCH_META_COUNTS_VERSION;
        save_branch_meta(&path, meta).expect("plant");
        assert!(list_native_sessions(dir.path()).expect("list").is_empty());
    }

    #[test]
    fn a_legacy_sidecar_is_listed_as_unknown_without_being_written() {
        let dir = tempdir();
        let path = dir.path().join("legacy.jsonl");
        std::fs::write(
            &path,
            b"not valid jsonl
",
        )
        .expect("write corrupt");
        let meta = BranchMeta {
            id: crate::sessionstore::branch::branch_id(&path.to_string_lossy()),
            created_at: crate::sessionstore::eventlog::record::CreatedAt::now(),
            updated_at: crate::sessionstore::eventlog::record::CreatedAt::now(),
            ..BranchMeta::default()
        };
        save_branch_meta(&path, meta).expect("write legacy meta");
        let before = load_branch_meta(&path).expect("meta io").expect("meta");

        let rows = list_native_sessions(dir.path()).expect("list");
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].counts_known);
        assert_eq!(rows[0].turns, 0);
        assert!(rows[0].preview.contains("being indexed"));
        // The listing never repairs the sidecar; that is the catalog worker's
        // job.
        let after = load_branch_meta(&path).expect("meta io").expect("meta");
        assert_eq!(after.schema_version, 0);
        assert_eq!(after.turns, 0);
        assert_eq!(after.updated_at, before.updated_at);
        assert_eq!(after.preview, before.preview);
    }

    #[test]
    fn an_unreadable_non_empty_session_stays_visible() {
        let dir = tempdir();
        let path = dir.path().join("corrupt.jsonl");
        std::fs::write(
            &path,
            b"not valid json
",
        )
        .expect("write corrupt");
        let rows = list_native_sessions(dir.path()).expect("list");
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].counts_known);
    }

    #[test]
    fn continue_session_path_keeps_one_file() {
        assert_eq!(
            continue_session_path("/sessions/existing.jsonl", "/sessions", "m"),
            "/sessions/existing.jsonl"
        );
        let fresh = continue_session_path("", "/sessions", "m");
        assert!(fresh.starts_with("/sessions"));
        assert!(fresh.ends_with("-m.jsonl"));
        assert_eq!(continue_session_path("", "", "m"), "");
    }

    #[test]
    fn title_source_joins_human_turns_and_clamps_to_300_runes() {
        let dir = tempdir();
        let path = dir.path().join("title.jsonl");
        let long = "x".repeat(400);
        let session = Session::new("sys");
        session.add(user("first ask"));
        session.add(assistant("answer"));
        session.add(user(&long));
        session.save(&path.to_string_lossy()).expect("save");
        let source = user_messages_title_source(&path).expect("title source");
        assert_eq!(source.chars().count(), 300);
        assert!(source.ends_with(&long[long.len() - 10..]));
        // A short history keeps every human turn, joined in order.
        assert!(user_messages_title_source(&dir.path().join("missing.jsonl")).is_err());
        let msgs = vec![user("one"), assistant("two"), user("three")];
        assert_eq!(
            user_messages_title_source_from_messages(&msgs),
            "one\nthree"
        );
    }

    #[test]
    fn disk_preview_swallows_decode_errors() {
        let dir = tempdir();
        let path = saved_session(dir.path(), "preview.jsonl", &["hello there"]);
        assert_eq!(session_preview(&path), ("hello there".to_string(), 1));
        let broken = dir.path().join("broken.jsonl");
        std::fs::write(&broken, b"{\n").expect("write broken");
        assert_eq!(session_preview(&broken), (String::new(), 0));
        // A cleanup-pending marker hides the row from an order listing.
        let pending = saved_session(dir.path(), "pending.jsonl", &["hidden"]);
        crate::sessionstore::session_order::mark_cleanup_pending(
            &pending.to_string_lossy(),
            "delete",
        )
        .expect("mark");
        let rows = list_native_sessions(dir.path()).expect("list");
        assert!(rows.iter().all(|row| row.path != pending.to_string_lossy()));
    }
}
