//! Session artifact naming.
//!
//! Parity target: `reference/.../internal/state/store/session.go`
//! (docs/contracts/session-format.md 1, 20.1).
//!
//! Every sidecar path is derived from the session transcript path. The
//! suffix list below is the single source of truth separating transcripts
//! from sidecars: a suffix missing from it would make a sidecar show up in
//! listings as a conversation of its own.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Suffixes that make a `.jsonl` file a JSONL sidecar rather than a
/// transcript; the last one is 1.x writing into the shared session dir.
///
/// Frozen by docs/contracts/session-format.md 1 (SESS-002): the Rust port
/// keeps one list, not a chain of conditions.
pub const JSONL_SIDECAR_SUFFIXES: [&str; 7] = [
    ".events.jsonl",
    ".conflicts.jsonl",
    ".guardian.jsonl",
    ".wire.jsonl",
    ".adjudication.jsonl",
    ".execution.jsonl",
    ".turns.jsonl",
];

const TRANSCRIPT_SUFFIX: &str = ".jsonl";

/// Strips one trailing `.jsonl` from `path`, leaving it
/// unchanged when it is not a transcript name (`sessionStem`,
/// session-format.md 1).
pub fn session_stem(path: &Path) -> Cow<'_, Path> {
    match strip_suffix(path, TRANSCRIPT_SUFFIX) {
        Some(stem) => Cow::Owned(stem.into_owned().into()),
        None => Cow::Borrowed(path),
    }
}

/// Reports whether `name` is a session transcript: trims whitespace,
/// must end in `.jsonl`, and must not end in any
/// [`JSONL_SIDECAR_SUFFIXES`] entry (`IsSessionTranscriptName`,
/// session-format.md 1).
pub fn is_session_transcript_name(name: &OsStr) -> bool {
    let lossy = name.to_string_lossy();
    let trimmed = lossy.trim();
    if !trimmed.ends_with(TRANSCRIPT_SUFFIX) {
        return false;
    }
    !JSONL_SIDECAR_SUFFIXES
        .iter()
        .any(|suffix| trimmed.ends_with(*suffix))
}

/// Reports whether `name` is a flat legacy subagent transcript: the
/// base name starts with `subagent-` (`IsSubagentTranscriptName`,
/// session-format.md 1). Excluded from user listings, still visible to
/// redaction, GC and migration.
pub fn is_subagent_transcript_name(name: &OsStr) -> bool {
    // `Path::file_name` matches `filepath.Base` for the relevant
    // inputs: "", ".", ".." and trailing separators yield no usable base
    // name, and none of those bases start with the prefix.
    match Path::new(name).file_name() {
        Some(base) => base.to_string_lossy().starts_with("subagent-"),
        None => false,
    }
}

/// Appends `suffix` to the full path, byte-exact (`OsString::push`).
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// `<id>.jsonl.meta` - branch/lineage metadata (session-format.md 1, 8).
/// Historical exception: appends to the FULL transcript path.
pub fn session_meta(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(session_path, ".meta")
}

/// `<id>.jsonl.lock` - advisory save lock (session-format.md 1).
pub fn session_lock_file(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(session_path, ".lock")
}

/// `<id>.jsonl.lease.lock` - runtime ownership lock (session-format.md 1, 9).
pub fn session_lease_lock(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(session_path, ".lease.lock")
}

/// `<id>.jsonl.lease.json` - runtime ownership metadata (1, 9).
pub fn session_lease_info(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(session_path, ".lease.json")
}

/// `<stem>.goal-state.json` - persisted active-goal sidecar (20.1).
pub fn session_goal_state(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".goal-state.json")
}

/// `<stem>.execution.jsonl` - append-only delegated-execution records,
/// written before work starts (session-format.md 1).
pub fn session_execution(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".execution.jsonl")
}

/// `<stem>.adjudication.jsonl` - append-only host adjudication
/// barriers, kept apart because the model never said it (session-format.md 1).
pub fn session_adjudication(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".adjudication.jsonl")
}

/// `<stem>.events.jsonl` - THE authoritative transcript event log
/// (session-format.md 1, 3).
pub fn session_event_log(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".events.jsonl")
}

/// `<stem>.wire.jsonl` - frontend event-frame log (1, 20.2).
pub fn session_wire_log(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".wire.jsonl")
}

/// `<stem>.wire.meta.json` - records what the wire log does not
/// contain (1, 20.2).
pub fn session_wire_log_meta(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".wire.meta.json")
}

/// `<stem>.events.jsonl.damaged` - forensic bytes tail repair dropped;
/// the suffix deliberately does not end in `.jsonl` so an older
/// binary's scan cannot resurrect it as a phantom session (1, 3.5).
pub fn session_event_log_damaged(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".events.jsonl.damaged")
}

/// `<stem>.event-index.json` - derived offsets/digests for listing
/// and the intact check (1, 3.6).
pub fn session_event_index(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".event-index.json")
}

/// `<stem>.display-index.json` - per-message byte offsets, roles, turn
/// boundaries; never bodies (1, 10).
pub fn session_display_index(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".display-index.json")
}

/// `<stem>.conflicts.jsonl` - append-only snapshot-conflict
/// diagnostics: revision counters and branch ids, not transcript content (1).
pub fn session_conflict_log(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".conflicts.jsonl")
}

/// `<stem>.recovery.json` - persisted Auto-mode recovery checkpoint
/// state (session-format.md 1).
pub fn session_recovery_state(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".recovery.json")
}

/// `<stem>.context.json` - context projection / compaction state plus
/// cache telemetry (1, 11).
pub fn session_context(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".context.json")
}

/// `<stem>.superseded.jsonl.bak` - what a leaseholder's overriding
/// write replaced; deliberately not `.jsonl` (session-format.md 1).
pub fn session_superseded(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".superseded.jsonl.bak")
}

/// `<stem>.ckpt` - snapshot checkpoints (1, 16).
pub fn session_checkpoint_dir(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".ckpt")
}

/// `<stem>.outputs` - tool output too large for context (1).
pub fn session_outputs_dir(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".outputs")
}

/// `<stem>.blobs` - image payloads the event log references by
/// content hash (1, 5).
pub fn session_blobs_dir(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".blobs")
}

/// `<stem>.jobs` - background-job artifacts (session-format.md 1).
pub fn session_jobs_dir(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".jobs")
}

/// `<stem>.inbox` - durable session-level instruction inbox (1, 17).
pub fn session_inbox_dir(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".inbox")
}

/// `<stem>.cleanup-pending.json` - delayed-cleanup marker (1, 7.13).
pub fn session_cleanup_pending(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(&session_stem(session_path), ".cleanup-pending.json")
}

/// `<stem>.v4-removed` - the durable tombstone of a deleted 1.x import, so
/// the v4 discovery pass never re-imports it (Go: `v4RemovalPath`,
/// session_v4_removal.go). Empty unless the name is `v4-<1.x id>.jsonl` in a
/// folder named `sessions` with a `sessions-v4` sibling.
pub fn session_v4_removed_marker(session_path: &Path) -> PathBuf {
    if session_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    let Some(name) = strip_suffix(session_path, ".jsonl") else {
        return PathBuf::new();
    };
    let Some(file_name) = Path::new(&name).file_name() else {
        return PathBuf::new();
    };
    let file_name = file_name.to_string_lossy().into_owned();
    let Some(id) = file_name.strip_prefix("v4-") else {
        return PathBuf::new();
    };
    // Go checks `filepath.Dir(sessionPath)` — the folder the transcript
    // lives in, not the transcript itself.
    let Some(dir) = Path::new(&name).parent() else {
        return PathBuf::new();
    };
    if !is_v4_session_id(id) || v4_sessions_root(dir).as_os_str().is_empty() {
        return PathBuf::new();
    }
    append_suffix(Path::new(&name), ".v4-removed")
}

/// The `sessions-v4` folder beside a `sessions` folder (Go: `v4RootFor`).
/// Empty for any other folder name, so the tombstone stays a sessions-folder
/// concept.
pub fn v4_sessions_root(sessions_dir: &Path) -> PathBuf {
    let is_sessions_folder = sessions_dir
        .file_name()
        .is_some_and(|name| name == "sessions");
    match (is_sessions_folder, sessions_dir.parent()) {
        (true, Some(parent)) if !parent.as_os_str().is_empty() => parent.join("sessions-v4"),
        _ => PathBuf::new(),
    }
}

/// Whether an id is one 1.x generates: lowercase hex, 16-64 digits, with or
/// without a Desktop marker prefix (Go: `isV4SessionID`).
pub fn is_v4_session_id(id: &str) -> bool {
    let mut rest = id;
    for prefix in ["desktop-manual-", "desktop-"] {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            rest = stripped;
            break;
        }
    }
    if !(16..=64).contains(&rest.len()) {
        return false;
    }
    rest.bytes()
        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Inverse of [`session_event_log`]: strips `.events.jsonl`,
/// requires a non-empty stem, appends `.jsonl`; yields the empty path
/// otherwise (`SessionPathForEventLog`, session-format.md 1, 20.1).
pub fn session_path_for_event_log(path: &Path) -> PathBuf {
    match strip_suffix(path, ".events.jsonl") {
        Some(stem) if !stem.is_empty() => {
            let mut session = stem.into_owned();
            session.push(".jsonl");
            PathBuf::from(session)
        }
        _ => PathBuf::new(),
    }
}

/// Inverse of [`session_outputs_dir`]: strips `.outputs` and
/// appends `.jsonl`; yields the empty path otherwise (20.1).
pub fn session_path_for_outputs_dir(path: &Path) -> PathBuf {
    match strip_suffix(path, ".outputs") {
        Some(stem) if !stem.is_empty() => {
            let mut session = stem.into_owned();
            session.push(".jsonl");
            PathBuf::from(session)
        }
        _ => PathBuf::new(),
    }
}

/// Derives `<stem>.blobs` from an event-log path; the empty path when
/// `path` is not an event log (`SessionBlobsDirForEventLog`, 20.1).
pub fn session_blobs_dir_for_event_log(path: &Path) -> PathBuf {
    match strip_suffix(path, ".events.jsonl") {
        Some(stem) if !stem.is_empty() => {
            let mut blobs = stem.into_owned();
            blobs.push(".blobs");
            PathBuf::from(blobs)
        }
        _ => PathBuf::new(),
    }
}

/// Regular-file sidecars a delete must remove, in the frozen order; empty
/// for a blank session path (`SessionSidecarFiles`, session-format.md
/// 20.1). Checkpoints, jobs and the inbox keep their own lifecycles and are
/// intentionally not in this list.
pub fn session_sidecar_files(session_path: &Path) -> Vec<PathBuf> {
    if is_blank(session_path) {
        return Vec::new();
    }
    vec![
        session_meta(session_path),
        session_goal_state(session_path),
        session_adjudication(session_path),
        session_execution(session_path),
        session_event_log(session_path),
        session_wire_log(session_path),
        session_wire_log_meta(session_path),
        session_event_log_damaged(session_path),
        session_event_index(session_path),
        session_display_index(session_path),
        session_conflict_log(session_path),
        session_recovery_state(session_path),
        session_context(session_path),
    ]
}

/// Sidecar directories removed with the session, in order; empty for a
/// blank path (`SessionSidecarDirs`, session-format.md 20.1).
pub fn session_sidecar_dirs(session_path: &Path) -> Vec<PathBuf> {
    if is_blank(session_path) {
        return Vec::new();
    }
    vec![
        session_outputs_dir(session_path),
        session_blobs_dir(session_path),
    ]
}

/// The ten sidecars a rename must carry over, as `(old, new)` pairs in
/// order (`sessionSidecarMoves`, session-format.md 20.1): goal state,
/// blobs, event log, event-log damaged, event index, conflicts, recovery,
/// checkpoints, jobs, inbox. Blobs move BEFORE the event log so a cut-short
/// rename never leaves a log whose images stayed behind. NOT moved: meta,
/// wire, wire meta, display index, execution, adjudication, outputs -
/// recomputed or rewritten instead.
pub fn session_sidecar_moves(old: &Path, new: &Path) -> Vec<(PathBuf, PathBuf)> {
    // Order matters: blobs must move before the event log (20.1).
    const MOVES: [&str; 10] = [
        ".goal-state.json",
        ".blobs",
        ".events.jsonl",
        ".events.jsonl.damaged",
        ".event-index.json",
        ".conflicts.jsonl",
        ".recovery.json",
        ".ckpt",
        ".jobs",
        ".inbox",
    ];
    let old_stem = session_stem(old);
    let new_stem = session_stem(new);
    MOVES
        .iter()
        .map(|suffix| {
            (
                append_suffix(&old_stem, suffix),
                append_suffix(&new_stem, suffix),
            )
        })
        .collect()
}

fn is_blank(path: &Path) -> bool {
    path.as_os_str().to_string_lossy().trim().is_empty()
}

/// `strings.CutSuffix` equivalent over raw path bytes.
///
/// On Unix the stem borrows the input; on Windows the trimmed UTF-16 form
/// must be rebuilt, so the return value may allocate. Lossy conversion is
/// deliberately avoided: a non-UTF-8 session directory must keep its exact
/// bytes through every sidecar path.
#[cfg(unix)]
fn strip_suffix<'a>(path: &'a Path, suffix: &str) -> Option<Cow<'a, OsStr>> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    let suffix_bytes = suffix.as_bytes();
    let stem_len = bytes.len().checked_sub(suffix_bytes.len())?;
    if &bytes[stem_len..] != suffix_bytes {
        return None;
    }
    Some(Cow::Borrowed(OsStr::from_bytes(&bytes[..stem_len])))
}

/// Windows twin of the Unix `strip_suffix` above: UTF-16 units instead
/// of bytes; the trimmed stem must be rebuilt, so the return value allocates.
#[cfg(windows)]
fn strip_suffix<'a>(path: &'a Path, suffix: &str) -> Option<Cow<'a, OsStr>> {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let path_units: Vec<u16> = path.as_os_str().encode_wide().collect();
    let suffix_units: Vec<u16> = suffix.encode_utf16().collect();
    let stem_len = path_units.len().checked_sub(suffix_units.len())?;
    if &path_units[stem_len..] != suffix_units.as_slice() {
        return None;
    }
    Some(Cow::Owned(OsString::from_wide(&path_units[..stem_len])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::path::Path;

    fn p(s: &str) -> &Path {
        Path::new(s)
    }

    #[test]
    fn session_stem_strips_exactly_one_transcript_suffix() {
        assert_eq!(&*session_stem(p("dir/sess.jsonl")), p("dir/sess"));
        assert_eq!(&*session_stem(p("sess.jsonl")), p("sess"));
        // Only the trailing suffix goes; the rest of the name is untouched.
        assert_eq!(&*session_stem(p("sess.jsonl.jsonl")), p("sess.jsonl"));
        assert_eq!(&*session_stem(p("sess.events.jsonl")), p("sess.events"));
        // Non-transcripts come back unchanged.
        assert_eq!(&*session_stem(p("sess.txt")), p("sess.txt"));
        assert_eq!(&*session_stem(p("")), p(""));
    }

    #[test]
    fn stem_accessors_append_to_the_stripped_stem() {
        let s = p("d/sess.jsonl");
        assert_eq!(
            session_goal_state(s),
            PathBuf::from("d/sess.goal-state.json")
        );
        assert_eq!(
            session_execution(s),
            PathBuf::from("d/sess.execution.jsonl")
        );
        assert_eq!(
            session_adjudication(s),
            PathBuf::from("d/sess.adjudication.jsonl")
        );
        assert_eq!(session_event_log(s), PathBuf::from("d/sess.events.jsonl"));
        assert_eq!(session_wire_log(s), PathBuf::from("d/sess.wire.jsonl"));
        assert_eq!(
            session_wire_log_meta(s),
            PathBuf::from("d/sess.wire.meta.json")
        );
        assert_eq!(
            session_event_log_damaged(s),
            PathBuf::from("d/sess.events.jsonl.damaged")
        );
        assert_eq!(
            session_event_index(s),
            PathBuf::from("d/sess.event-index.json")
        );
        assert_eq!(
            session_display_index(s),
            PathBuf::from("d/sess.display-index.json")
        );
        assert_eq!(
            session_conflict_log(s),
            PathBuf::from("d/sess.conflicts.jsonl")
        );
        assert_eq!(
            session_recovery_state(s),
            PathBuf::from("d/sess.recovery.json")
        );
        assert_eq!(session_context(s), PathBuf::from("d/sess.context.json"));
        assert_eq!(
            session_superseded(s),
            PathBuf::from("d/sess.superseded.jsonl.bak")
        );
        assert_eq!(session_checkpoint_dir(s), PathBuf::from("d/sess.ckpt"));
        assert_eq!(session_outputs_dir(s), PathBuf::from("d/sess.outputs"));
        assert_eq!(session_blobs_dir(s), PathBuf::from("d/sess.blobs"));
        assert_eq!(session_jobs_dir(s), PathBuf::from("d/sess.jobs"));
        assert_eq!(session_inbox_dir(s), PathBuf::from("d/sess.inbox"));
        assert_eq!(
            session_cleanup_pending(s),
            PathBuf::from("d/sess.cleanup-pending.json")
        );
    }

    #[test]
    fn historical_accessors_append_to_the_full_transcript_path() {
        let s = p("d/sess.jsonl");
        assert_eq!(session_meta(s), PathBuf::from("d/sess.jsonl.meta"));
        assert_eq!(session_lock_file(s), PathBuf::from("d/sess.jsonl.lock"));
        assert_eq!(
            session_lease_lock(s),
            PathBuf::from("d/sess.jsonl.lease.lock")
        );
        assert_eq!(
            session_lease_info(s),
            PathBuf::from("d/sess.jsonl.lease.json")
        );
    }

    #[test]
    fn jsonl_sidecar_suffixes_are_the_frozen_seven() {
        assert_eq!(
            JSONL_SIDECAR_SUFFIXES,
            [
                ".events.jsonl",
                ".conflicts.jsonl",
                ".guardian.jsonl",
                ".wire.jsonl",
                ".adjudication.jsonl",
                ".execution.jsonl",
                ".turns.jsonl",
            ]
        );
    }

    #[test]
    fn transcript_predicate_accepts_plain_transcripts_only() {
        for name in [
            "a.jsonl",
            " a.jsonl ",
            "a.jsonl.jsonl",
            "events.jsonl",
            "wire.jsonl",
            "模型.jsonl",
            ".jsonl",
        ] {
            assert!(is_session_transcript_name(OsStr::new(name)), "{name:?}");
        }
        for name in ["a.txt", "a", "", " "] {
            assert!(!is_session_transcript_name(OsStr::new(name)), "{name:?}");
        }
        for suffix in JSONL_SIDECAR_SUFFIXES {
            assert!(!is_session_transcript_name(OsStr::new(&format!(
                "a{suffix}"
            ))));
            assert!(!is_session_transcript_name(OsStr::new(&format!(
                "a{suffix} "
            ))));
        }
    }

    #[test]
    fn subagent_predicate_requires_prefix_on_the_base_name() {
        for name in [
            "subagent-x.jsonl",
            "dir/subagent-y",
            "subagent-",
            "a/subagent-z.jsonl",
        ] {
            assert!(is_subagent_transcript_name(OsStr::new(name)), "{name:?}");
        }
        for name in [
            "x.jsonl",
            "subagentless.jsonl",
            "subagent",
            "",
            ".",
            "..",
            "/",
        ] {
            assert!(!is_subagent_transcript_name(OsStr::new(name)), "{name:?}");
        }
    }

    #[test]
    fn inverters_recover_session_paths() {
        assert_eq!(
            session_path_for_event_log(p("d/s.events.jsonl")),
            PathBuf::from("d/s.jsonl")
        );
        assert_eq!(
            session_path_for_event_log(p("s.events.jsonl")),
            PathBuf::from("s.jsonl")
        );
        // A directory-only stem is still a stem; a bare suffix is not.
        assert_eq!(
            session_path_for_event_log(p("d/.events.jsonl")),
            PathBuf::from("d/.jsonl")
        );
        assert_eq!(
            session_path_for_event_log(p(".events.jsonl")),
            PathBuf::new()
        );
        assert_eq!(session_path_for_event_log(p("d/s.jsonl")), PathBuf::new());
        assert_eq!(session_path_for_event_log(p("")), PathBuf::new());

        assert_eq!(
            session_path_for_outputs_dir(p("d/s.outputs")),
            PathBuf::from("d/s.jsonl")
        );
        assert_eq!(session_path_for_outputs_dir(p("d/s")), PathBuf::new());
        assert_eq!(session_path_for_outputs_dir(p(".outputs")), PathBuf::new());

        assert_eq!(
            session_blobs_dir_for_event_log(p("d/s.events.jsonl")),
            PathBuf::from("d/s.blobs")
        );
        assert_eq!(
            session_blobs_dir_for_event_log(p("d/s.jsonl")),
            PathBuf::new()
        );
    }

    #[test]
    fn sidecar_file_list_is_the_frozen_thirteen_in_order() {
        let files = session_sidecar_files(p("d/s.jsonl"));
        assert_eq!(files.len(), 13);
        assert_eq!(
            files,
            [
                "d/s.jsonl.meta",
                "d/s.goal-state.json",
                "d/s.adjudication.jsonl",
                "d/s.execution.jsonl",
                "d/s.events.jsonl",
                "d/s.wire.jsonl",
                "d/s.wire.meta.json",
                "d/s.events.jsonl.damaged",
                "d/s.event-index.json",
                "d/s.display-index.json",
                "d/s.conflicts.jsonl",
                "d/s.recovery.json",
                "d/s.context.json",
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn blank_session_paths_yield_empty_sidecar_lists() {
        for blank in ["", "   "] {
            assert!(session_sidecar_files(p(blank)).is_empty());
            assert!(session_sidecar_dirs(p(blank)).is_empty());
        }
        assert_eq!(
            session_sidecar_dirs(p("d/s.jsonl")),
            [PathBuf::from("d/s.outputs"), PathBuf::from("d/s.blobs")]
        );
    }

    #[test]
    fn blank_session_paths_yield_empty_accessor_paths() {
        // Go's session accessors return the zero path for a blank input;
        // the Rust port mirrors that with an empty PathBuf so callers
        // never derive a sidecar path in the current directory.
        let blank = Path::new("");
        for empty in [
            session_meta(blank),
            session_lock_file(blank),
            session_lease_lock(blank),
            session_lease_info(blank),
            session_goal_state(blank),
            session_execution(blank),
            session_adjudication(blank),
            session_event_log(blank),
            session_wire_log(blank),
            session_wire_log_meta(blank),
            session_event_log_damaged(blank),
            session_event_index(blank),
            session_display_index(blank),
            session_conflict_log(blank),
            session_recovery_state(blank),
            session_context(blank),
            session_superseded(blank),
            session_checkpoint_dir(blank),
            session_outputs_dir(blank),
            session_blobs_dir(blank),
            session_jobs_dir(blank),
            session_inbox_dir(blank),
            session_cleanup_pending(blank),
        ] {
            assert_eq!(empty, PathBuf::new());
        }
    }

    #[test]
    fn sidecar_moves_are_the_frozen_ten_pairs_in_order() {
        let moves = session_sidecar_moves(p("d/old.jsonl"), p("d/new.jsonl"));
        assert_eq!(moves.len(), 10);
        assert_eq!(
            moves,
            [
                (p("d/old.goal-state.json"), p("d/new.goal-state.json")),
                (p("d/old.blobs"), p("d/new.blobs")),
                (p("d/old.events.jsonl"), p("d/new.events.jsonl")),
                (
                    p("d/old.events.jsonl.damaged"),
                    p("d/new.events.jsonl.damaged")
                ),
                (p("d/old.event-index.json"), p("d/new.event-index.json")),
                (p("d/old.conflicts.jsonl"), p("d/new.conflicts.jsonl")),
                (p("d/old.recovery.json"), p("d/new.recovery.json")),
                (p("d/old.ckpt"), p("d/new.ckpt")),
                (p("d/old.jobs"), p("d/new.jobs")),
                (p("d/old.inbox"), p("d/new.inbox")),
            ]
            .into_iter()
            .map(|(old, new)| (old.to_path_buf(), new.to_path_buf()))
            .collect::<Vec<_>>()
        );
        // Blobs must move before the event log (session-format.md 20.1).
        let blobs = moves
            .iter()
            .position(|(old, _)| old.to_string_lossy().ends_with(".blobs"))
            .unwrap();
        let event_log = moves
            .iter()
            .position(|(old, _)| old.to_string_lossy().ends_with(".events.jsonl"))
            .unwrap();
        assert!(blobs < event_log);
        // Recomputed or rewritten sidecars are deliberately not moved.
        for survivor in [
            "d/old.jsonl.meta",
            "d/old.wire.jsonl",
            "d/old.wire.meta.json",
            "d/old.display-index.json",
            "d/old.execution.jsonl",
            "d/old.adjudication.jsonl",
            "d/old.outputs",
        ] {
            assert!(
                !moves.iter().any(|(old, _)| old == p(survivor)),
                "{survivor}"
            );
        }
    }
}
