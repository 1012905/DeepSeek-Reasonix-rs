//! Rewind versions: superseded conversations kept as whole session files.
//!
//! Parity target: `internal/state/sessionstore/versions.go`
//! (`docs/contracts/session-format.md` 2, ledger SESS-005).
//!
//! A version is a conversation as it stood before a rewind replaced part
//! of it. It is a whole session file and opens like one; it only stays out
//! of the session lists, where it would read as a second conversation.
//!
//! Not ported from the Go source:
//! - the `knownNotSuperseded` listing-index fast path (SESS-030): without
//!   a remembered listing every transcript's branch meta is read, exactly
//!   what Go does in a folder the index does not cover; the observable
//!   grouping is identical.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use reasonix_contract::provider::Message;
use thiserror::Error;

use super::branch::{
    BRANCH_META_COUNTS_VERSION, BranchMeta, BranchMetaError, branch_id, load_branch_meta,
    save_branch_meta,
};
use super::eventlog::record::{CreatedAt, now_utc};
use super::path::new_session_path;
use super::session::{LoadError, SaveError, Session, is_visible_session, load_session};
use super::sessionpreview::session_preview_from_messages;
use crate::store::{RemoveArtifactsError, is_session_transcript_name, remove_session_artifacts};

/// Errors raised while keeping, finding, or dropping rewind versions.
#[derive(Debug, Error)]
pub enum VersionsError {
    #[error(transparent)]
    Save(#[from] SaveError),
    #[error(transparent)]
    Meta(#[from] BranchMetaError),
    #[error(transparent)]
    Load(#[from] LoadError),
    #[error(transparent)]
    Remove(#[from] RemoveArtifactsError),
}

/// A conversation as it stood before a rewind replaced part of it
/// (Go `sessionstore.SessionVersion`).
#[derive(Debug, Clone)]
pub struct SessionVersion {
    pub path: String,
    pub turns: i64,
    pub preview: String,
    pub saved_at: CreatedAt,
}

/// Go `SaveSupersededVersion`: keep `msgs`, the conversation a rewind is
/// about to cut, as a version of the session at `parent_path`, and return
/// where it went.
pub fn save_superseded_version(
    parent_path: &str,
    msgs: &[Message],
) -> Result<String, VersionsError> {
    // The same whole-session construction the archive decoder uses
    // (Go: NewSession + Messages assignment).
    let session = Session::from_archive_records(msgs.to_vec());
    let dir = parent_dir_of(parent_path);
    let path = new_session_path(&dir, "version");
    let path_str = path.to_string_lossy().into_owned();
    session.save_if_absent(&path_str)?;

    let (preview, turns) = session_preview_from_messages(msgs);
    let mut meta = BranchMeta {
        parent_id: branch_id(parent_path),
        superseded: true,
        preview,
        turns,
        schema_version: BRANCH_META_COUNTS_VERSION,
        created_at: now_utc(),
        ..BranchMeta::default()
    };
    if let Ok(Some(parent)) = load_branch_meta(Path::new(parent_path)) {
        meta.scope = parent.scope;
        meta.workspace_root = parent.workspace_root;
        meta.model = parent.model;
        meta.agent_preset = parent.agent_preset;
        meta.tool_approval_mode = parent.tool_approval_mode;
    }
    if let Err(err) = save_branch_meta(&path, meta) {
        let _ = std::fs::remove_file(&path);
        return Err(err.into());
    }
    Ok(path_str)
}

/// Go `ListSessionVersions`: group the versions under `dir` by the
/// conversation they were cut from, newest first. A missing directory is
/// not an error.
pub fn list_session_versions(dir: &Path) -> io::Result<HashMap<String, Vec<SessionVersion>>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(HashMap::new());
        }
        Err(err) => return Err(err),
    };
    let mut out: HashMap<String, Vec<SessionVersion>> = HashMap::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() || !is_session_transcript_name(&entry.file_name()) {
            continue;
        }
        let full = dir.join(entry.file_name());
        let full_str = full.to_string_lossy().into_owned();
        // Go skips load errors and absent sidecars alike.
        let Ok(Some(meta)) = load_branch_meta(&full) else {
            continue;
        };
        if !meta.superseded || meta.parent_id.is_empty() || !is_visible_session(&full_str) {
            continue;
        }
        out.entry(meta.parent_id.clone())
            .or_default()
            .push(SessionVersion {
                path: full_str,
                turns: meta.turns,
                preview: meta.preview,
                saved_at: meta.created_at,
            });
    }
    for versions in out.values_mut() {
        // Newest first (Go sorts by SavedAt.After).
        versions.sort_by_key(|a| std::cmp::Reverse(a.saved_at.get()));
    }
    Ok(out)
}

/// Go `SessionVersionPaths`: the versions cut from the session at
/// `parent_path`, for a caller removing it: a version nobody lists must
/// not outlive its parent.
pub fn session_version_paths(parent_path: &str) -> Vec<String> {
    let dir = parent_dir_of(parent_path);
    let by_parent = list_session_versions(&dir).unwrap_or_default();
    match by_parent.get(&branch_id(parent_path)) {
        Some(versions) => versions.iter().map(|v| v.path.clone()).collect(),
        None => Vec::new(),
    }
}

/// Go `DropVersionMatching`: remove the newest version of the session at
/// `parent_path` when it holds exactly `msgs`: undoing a rewind puts back
/// the conversation that rewind kept, and leaving the copy would show
/// one conversation twice.
pub fn drop_version_matching(parent_path: &str, msgs: &[Message]) -> Result<(), VersionsError> {
    let versions = session_version_paths(parent_path);
    let Some(newest) = versions.first() else {
        return Ok(());
    };
    let kept = load_session(newest)?;
    if !same_messages(&kept.snapshot(), msgs) {
        return Ok(());
    }
    Ok(remove_session_artifacts(
        Path::new(newest),
        &[] as &[&Path],
    )?)
}

/// Go `sameMessages`: role, content, and tool-call count only.
fn same_messages(a: &[Message], b: &[Message]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.role == y.role && x.content == y.content && x.tool_calls.len() == y.tool_calls.len()
        })
}

/// Go `filepath.Dir`: the parent directory, or `.` when there is none.
fn parent_dir_of(path: &str) -> std::path::PathBuf {
    Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::super::branch::{
        branch_id, load_branch_meta, update_branch_meta, update_session_meta,
    };
    use super::super::eventlog::test_support::tempdir;
    use super::*;
    use reasonix_contract::provider::Role;

    fn user_message(content: &str) -> Message {
        Message {
            role: Role::USER,
            content: content.to_string(),
            ..Message::default()
        }
    }

    fn assistant_message(content: &str) -> Message {
        Message {
            role: Role::ASSISTANT,
            content: content.to_string(),
            ..Message::default()
        }
    }

    fn two_turn_messages() -> Vec<Message> {
        vec![
            user_message("first"),
            assistant_message("one"),
            user_message("second"),
            assistant_message("two"),
        ]
    }

    #[test]
    fn superseded_versions_carry_parent_meta_and_preview() {
        let dir = tempdir();
        let parent = new_session_path(dir.path(), "test");
        let parent_str = parent.to_string_lossy().into_owned();
        let parent_session = Session::from_archive_records(vec![user_message("parent turn")]);
        parent_session
            .save_if_absent(&parent_str)
            .expect("parent save");
        update_branch_meta(&parent, false, |meta| {
            meta.scope = "scope-x".to_string();
            meta.workspace_root = "ws-root".to_string();
            meta.model = "model-x".to_string();
            meta.agent_preset = "preset-x".to_string();
            meta.tool_approval_mode = "auto".to_string();
            Ok(())
        })
        .expect("parent meta");

        let msgs = two_turn_messages();
        let version = save_superseded_version(&parent_str, &msgs).expect("version saved");
        assert!(
            version.ends_with("-version.jsonl"),
            "version name: {version}"
        );
        assert!(Path::new(&version).exists());

        let meta = load_branch_meta(Path::new(&version))
            .expect("meta io")
            .expect("meta present");
        assert!(meta.superseded);
        assert_eq!(meta.parent_id, branch_id(&parent_str));
        assert_eq!(meta.preview, "first");
        assert_eq!(meta.turns, 2);
        assert_eq!(meta.schema_version, BRANCH_META_COUNTS_VERSION);
        assert_eq!(meta.scope, "scope-x");
        assert_eq!(meta.workspace_root, "ws-root");
        assert_eq!(meta.model, "model-x");
        assert_eq!(meta.agent_preset, "preset-x");
        assert_eq!(meta.tool_approval_mode, "auto");

        let transcript = std::fs::read_to_string(Path::new(&version)).expect("read version");
        assert_eq!(transcript.lines().count(), 4);
        let reloaded = load_session(&version).expect("reload version");
        assert_eq!(reloaded.snapshot().len(), 4);
    }

    #[test]
    fn drop_version_matching_leaves_a_different_version() {
        let dir = tempdir();
        let parent = new_session_path(dir.path(), "test");
        let parent_str = parent.to_string_lossy().into_owned();
        let msgs = two_turn_messages();
        save_superseded_version(&parent_str, &msgs).expect("version saved");

        drop_version_matching(&parent_str, &msgs[..2]).expect("drop attempt");
        assert_eq!(
            session_version_paths(&parent_str).len(),
            1,
            "a non-matching restore dropped the version"
        );

        drop_version_matching(&parent_str, &msgs).expect("drop");
        let remaining = session_version_paths(&parent_str);
        assert!(
            remaining.is_empty(),
            "the matching version survived: {remaining:?}"
        );
    }

    #[test]
    fn versions_group_by_parent_newest_first() {
        let dir = tempdir();
        let parent = new_session_path(dir.path(), "test");
        let parent_str = parent.to_string_lossy().into_owned();
        let v1 = save_superseded_version(&parent_str, &[user_message("first take")]).expect("v1");
        std::thread::sleep(std::time::Duration::from_millis(20));
        let v2 = save_superseded_version(&parent_str, &[user_message("later take")]).expect("v2");
        assert_ne!(v1, v2);

        let other_parent = new_session_path(dir.path(), "other");
        let other_str = other_parent.to_string_lossy().into_owned();
        save_superseded_version(&other_str, &[user_message("elsewhere")]).expect("v3");

        let by_parent = list_session_versions(dir.path()).expect("list versions");
        let mine = by_parent
            .get(&branch_id(&parent_str))
            .expect("parent grouped");
        assert_eq!(mine.len(), 2);
        assert_eq!(mine[0].path, v2, "newest first");
        assert_eq!(mine[1].path, v1);
        assert_eq!(mine[0].preview, "later take");
        assert_eq!(mine[0].turns, 1);
        assert_eq!(
            by_parent
                .get(&branch_id(&other_str))
                .expect("other parent grouped")
                .len(),
            1
        );

        let missing =
            list_session_versions(&dir.path().join("nope")).expect("missing dir is not an error");
        assert!(missing.is_empty());
    }

    #[test]
    fn carried_on_versions_stop_listing_as_versions() {
        let dir = tempdir();
        let parent = new_session_path(dir.path(), "test");
        let parent_str = parent.to_string_lossy().into_owned();
        let version =
            save_superseded_version(&parent_str, &two_turn_messages()).expect("version saved");
        assert_eq!(session_version_paths(&parent_str).len(), 1);

        // A save that added no turn does not promote the version.
        update_session_meta(Path::new(&version), "", "second".to_string(), 2, false)
            .expect("meta update");
        assert_eq!(
            session_version_paths(&parent_str).len(),
            1,
            "a save that added no turn promoted the version"
        );

        // Carrying the version on turns it back into a conversation.
        update_session_meta(Path::new(&version), "", "third".to_string(), 3, true)
            .expect("meta update");
        assert!(
            session_version_paths(&parent_str).is_empty(),
            "a carried-on version still lists as a version"
        );
    }
}
