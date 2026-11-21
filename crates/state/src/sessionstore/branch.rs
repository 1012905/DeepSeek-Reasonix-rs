//! Branch metadata: the `.meta` sidecar, its persistence rules,
//! and the per-session revision ledger.
//!
//! Parity target: reference/.../internal/state/sessionstore/branch.go,
//! plus the revision-ledger helpers that live in save.go there
//! (`sessionContentRevision`, `recordSessionContentRevision`,
//! `LockSessionMetaPath`, `sessionMetaLockPath`) and Go's
//! `fmt.Sprintf("%v")` float formatting for the rank guard
//! (docs/contracts/session-format.md 8).
//!
//! The sidecar is metadata-only: listing, sorting, and the revision
//! ledger live here; message bodies never do. The revision counter is
//! monotonic — `preserve_branch_meta_persistence` blocks regression
//! whenever a stale snapshot races a newer writer, and the in-flight-turn
//! marker survives plain rewrites so a crashed turn can still be
//! adjudicated on restart.
//!
//! Not ported: the `encoding/json` error text itself (the Rust
//! side reports serde's message inside the same `decode branch meta
//! <path>: <err>` wrapper); and a `NaN` `manual_rank`
//! serializes as JSON `null` instead of failing the marshal —
//! `set_session_rank` rejects non-finite ranks, so no in-tree
//! writer can produce one.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::Utc;
use reasonix_contract::go_struct;
use reasonix_contract::gojson::{escape_html, go_vec, is_zero_bool, is_zero_i64, null_default};
use sha2::{Digest, Sha256};

use super::eventlog::record::{CreatedAt, deserialize_created_at, now_utc};
use super::eventlog::writer_id::{digest_string, session_writer_id};
use super::preview::user_preview_text;
use super::session::canonical_session_save_path;
use crate::store::naming;
use reasonix_core::fileencoding::decode_to_utf8;
use reasonix_core::filelock::{self, FileLockError, FileLockGuard};
use reasonix_core::fileutil::{create_temp_file, mkdir_all_mode, replace_file};

/// Schema version stamped by the counts migration (Go:
/// `branchMetaCountsVersion`; `branchMetaCountsInitialVersion`
/// is implicit — the zero value).
pub const BRANCH_META_COUNTS_VERSION: i64 = 2;

/// Cross-process wait for the per-session metadata lock (Go:
/// `sessionMetaLockWait`).
const SESSION_META_LOCK_WAIT: Duration = Duration::from_secs(5);

/// Read backoffs around the tolerant sidecar load (Go:
/// `branchMetaReadBackoffs`): Windows `ReplaceFile` can tear a
/// concurrent reader's view of the sidecar, so retries beat failing a
/// save.
const BRANCH_META_READ_BACKOFFS: [Duration; 3] = [
    Duration::from_millis(20),
    Duration::from_millis(50),
    Duration::from_millis(100),
];

/// Process-global sequence for in-flight-turn markers (Go:
/// `inFlightTurnSequence`).
static IN_FLIGHT_TURN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Test-only hook fired at the top of every [`load_branch_meta`] call
/// (Go: `branchMetaReadHook`); the listing tests observe sidecar reads
/// through it. Production builds compile it away.
#[cfg(test)]
pub(crate) static BRANCH_META_READ_HOOK: std::sync::Mutex<Option<fn(&Path)>> =
    std::sync::Mutex::new(None);

/// `skip_serializing_if` helper: Go's `omitempty` drops a
/// `0.0` rank.
pub(crate) fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

go_struct! {
    /// Metadata about an in-flight turn (Go: `inFlightTurnMeta`).
    pub struct InFlightTurnMeta : InFlightTurnMetaWire {
        #[serde(
            rename = "id",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub id: String,
        #[serde(rename = "start_message_index", default, deserialize_with = "null_default")]
        pub start_message_index: i64,
        #[serde(rename = "preserve_user", default, deserialize_with = "null_default")]
        pub preserve_user: bool,
        #[serde(rename = "started_at", default, deserialize_with = "deserialize_created_at")]
        pub started_at: CreatedAt,
        #[serde(
            rename = "start_revision",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub start_revision: i64,
        #[serde(
            rename = "start_digest",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub start_digest: String,
        #[serde(
            rename = "commit_digest",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub commit_digest: String,
    }
}

go_struct! {
    /// Where a legacy 1.x session was imported from (Go:
    /// `importSource`).
    pub struct ImportSource : ImportSourceWire {
        #[serde(rename = "line", default, deserialize_with = "null_default")]
        pub line: String,
        #[serde(rename = "log_size", default, deserialize_with = "null_default")]
        pub log_size: i64,
        #[serde(rename = "digest", default, deserialize_with = "null_default")]
        pub digest: String,
    }
}

go_struct! {
    /// Record of a conversation adoption (Go: `adoptRecord`).
    pub struct AdoptRecord : AdoptRecordWire {
        #[serde(rename = "kind", default, deserialize_with = "null_default")]
        pub kind: String,
        #[serde(
            rename = "from",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub from: String,
        #[serde(
            rename = "from_root",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub from_root: String,
        #[serde(rename = "at", default, deserialize_with = "deserialize_created_at")]
        pub at: CreatedAt,
    }
}

// Four boolean flags mirror the Go struct field-for-field; collapsing
// them would drift the wire format (AGENTS.md 3.2).
go_struct! {
    /// Per-session branch metadata persisted as the `.meta` sidecar
    /// (Go: `BranchMeta`). Field order and `omitempty` matrix
    /// are wire format: the file is read back by both kernels and the
    /// digest fields feed the save pipeline's CAS checks.
    #[allow(clippy::struct_excessive_bools)]
    pub struct BranchMeta : BranchMetaWire {
        #[serde(rename = "id", default, deserialize_with = "null_default")]
        pub id: String,
        #[serde(
            rename = "name",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub name: String,
        #[serde(
            rename = "parent_id",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub parent_id: String,
        #[serde(rename = "created_at", default, deserialize_with = "deserialize_created_at")]
        pub created_at: CreatedAt,
        #[serde(rename = "updated_at", default, deserialize_with = "deserialize_created_at")]
        pub updated_at: CreatedAt,
        #[serde(
            rename = "scope",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub scope: String,
        #[serde(
            rename = "workspace_root",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub workspace_root: String,
        #[serde(
            rename = "topic_id",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub topic_id: String,
        #[serde(
            rename = "topic_title",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub topic_title: String,
        #[serde(
            rename = "custom_title",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub custom_title: String,
        #[serde(
            rename = "archived",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub archived: bool,
        #[serde(
            rename = "pinned",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub pinned: bool,
        #[serde(
            rename = "manual_rank",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_f64"
        )]
        pub manual_rank: f64,
        #[serde(
            rename = "auto_archived_at",
            default,
            deserialize_with = "deserialize_created_at",
            skip_serializing_if = "CreatedAt::is_zero"
        )]
        pub auto_archived_at: CreatedAt,
        #[serde(
            rename = "finished_at",
            default,
            deserialize_with = "deserialize_created_at",
            skip_serializing_if = "CreatedAt::is_zero"
        )]
        pub finished_at: CreatedAt,
        #[serde(
            rename = "viewed_at",
            default,
            deserialize_with = "deserialize_created_at",
            skip_serializing_if = "CreatedAt::is_zero"
        )]
        pub viewed_at: CreatedAt,
        #[serde(
            rename = "model",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub model: String,
        #[serde(
            rename = "token_mode",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub token_mode: String,
        #[serde(
            rename = "agent_preset",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub agent_preset: String,
        #[serde(
            rename = "mode",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub mode: String,
        #[serde(
            rename = "tool_approval_mode",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub tool_approval_mode: String,
        #[serde(
            rename = "session_grants",
            default,
            deserialize_with = "go_vec",
            skip_serializing_if = "Vec::is_empty"
        )]
        pub session_grants: Vec<String>,
        #[serde(
            rename = "goal",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub goal: String,
        #[serde(
            rename = "recovered",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub recovered: bool,
        #[serde(
            rename = "recovery_reason",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub recovery_reason: String,
        #[serde(
            rename = "recovery_digest",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub recovery_digest: String,
        #[serde(
            rename = "recovery_depth",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub recovery_depth: i64,
        #[serde(
            rename = "recovery_root_id",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub recovery_root_id: String,
        #[serde(
            rename = "superseded",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub superseded: bool,
        #[serde(
            rename = "revision",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub revision: i64,
        #[serde(
            rename = "content_digest",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub content_digest: String,
        #[serde(
            rename = "writer_id",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub writer_id: String,
        #[serde(
            rename = "schema_version",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub schema_version: i64,
        #[serde(
            rename = "turns",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub turns: i64,
        #[serde(
            rename = "preview",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub preview: String,
        #[serde(
            rename = "in_flight_turn",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        pub in_flight_turn: Option<InFlightTurnMeta>,
        #[serde(
            rename = "imported_from",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        pub imported_from: Option<ImportSource>,
        #[serde(
            rename = "adopted",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        pub adopted: Option<AdoptRecord>,
    }
}

/// Errors from branch-metadata operations.
#[derive(Debug, thiserror::Error)]
pub enum BranchMetaError {
    #[error("empty session path")]
    EmptyPath,
    #[error("create session metadata dir: {0}")]
    CreateMetaDir(#[source] io::Error),
    #[error("lock session metadata: {0}")]
    LockSessionMeta(#[source] FileLockError),
    #[error("decode branch meta {meta_path}: {source}")]
    DecodeBranchMeta {
        meta_path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("a session rank is a finite number from 0, not {0}")]
    RankInvalid(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

// ---------------------------------------------------------------------------
// Path helpers (Go 1.26.11 internal/filepathlite, Windows rules; the
// reference toolchain pins go1.26.11, whose volume handling differs
// from newer stdlib).
// ---------------------------------------------------------------------------

/// Go 1.26.11 `filepathlite.IsPathSeparator` (Windows): both
/// separators count everywhere.
fn is_path_separator(c: u8) -> bool {
    c == b'\\' || c == b'/'
}

/// Go 1.26.11 `filepathlite.toUpper`: ASCII only.
fn to_upper(c: u8) -> u8 {
    if c.is_ascii_lowercase() {
        c - b'a' + b'A'
    } else {
        c
    }
}

/// Go 1.26.11 `filepathlite.pathHasPrefixFold`: case-insensitive
/// prefix match treating all separators as equivalent; a longer `s`
/// must continue with a separator.
fn path_has_prefix_fold(s: &[u8], prefix: &[u8]) -> bool {
    if s.len() < prefix.len() {
        return false;
    }
    for i in 0..prefix.len() {
        if is_path_separator(prefix[i]) {
            if !is_path_separator(s[i]) {
                return false;
            }
        } else if to_upper(prefix[i]) != to_upper(s[i]) {
            return false;
        }
    }
    if s.len() > prefix.len() && !is_path_separator(s[prefix.len()]) {
        return false;
    }
    true
}

/// Go 1.26.11 `filepathlite.cutPath`: the tail after the first
/// separator.
fn cut_path(path: &[u8]) -> Option<&[u8]> {
    for (i, &c) in path.iter().enumerate() {
        if is_path_separator(c) {
            return Some(&path[i + 1..]);
        }
    }
    None
}

/// Go 1.26.11 `filepathlite.uncLen`: index of the second separator
/// at or after `prefix_len`, or the full length.
fn unc_len(path: &[u8], prefix_len: usize) -> usize {
    let mut count = 0;
    for (i, byte) in path.iter().enumerate().skip(prefix_len) {
        if is_path_separator(*byte) {
            count += 1;
            if count == 2 {
                return i;
            }
        }
    }
    path.len()
}

/// Go 1.26.11 `filepathlite.volumeNameLen` (Windows): drive
/// letters, `\\.\UNC` host+share, device prefixes, and plain UNC
/// roots all count as volume.
fn volume_name_len(path: &str) -> usize {
    let b = path.as_bytes();
    if b.len() >= 2 && b[1] == b':' {
        return 2;
    }
    if b.is_empty() || !is_path_separator(b[0]) {
        return 0;
    }
    const UNC_DEVICE_PREFIX: &[u8] = b"\\\\.\\UNC";
    if path_has_prefix_fold(b, UNC_DEVICE_PREFIX) {
        return unc_len(b, UNC_DEVICE_PREFIX.len() + 1);
    }
    if path_has_prefix_fold(b, b"\\\\.")
        || path_has_prefix_fold(b, b"\\\\?")
        || path_has_prefix_fold(b, b"\\??")
    {
        if b.len() == 3 {
            return 3;
        }
        return match cut_path(&b[4..]) {
            Some(rest) => b.len() - rest.len() - 1,
            None => b.len(),
        };
    }
    if b.len() >= 2 && is_path_separator(b[1]) {
        return unc_len(b, 2);
    }
    0
}

/// Go 1.26.11 `filepathlite.Base` (Windows).
pub(crate) fn path_base(path: &str) -> &str {
    if path.is_empty() {
        return ".";
    }
    let b = path.as_bytes();
    let mut end = b.len();
    while end > 0 && is_path_separator(b[end - 1]) {
        end -= 1;
    }
    let stripped = &path[..end];
    let vol = volume_name_len(stripped);
    let rest = &stripped.as_bytes()[vol..];
    let mut i = rest.len() as isize - 1;
    while i >= 0 && !is_path_separator(rest[i as usize]) {
        i -= 1;
    }
    let out = if i >= 0 {
        &stripped[vol + i as usize + 1..]
    } else {
        &stripped[vol..]
    };
    if out.is_empty() {
        WINDOWS_SEPARATOR
    } else {
        out
    }
}

/// Go 1.26.11 `filepathlite.Ext`.
fn path_ext(path: &str) -> &str {
    let b = path.as_bytes();
    for i in (0..b.len()).rev() {
        if is_path_separator(b[i]) {
            return "";
        }
        if b[i] == b'.' {
            return &path[i..];
        }
    }
    ""
}

/// Go Windows `filepath.Separator` as a string.
const WINDOWS_SEPARATOR: &str = "\\";

/// Go branch.go `BranchID`: the transcript stem.
pub fn branch_id(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let base = path_base(path);
    let ext = path_ext(base);
    if ext.is_empty() {
        base.to_string()
    } else {
        base[..base.len() - ext.len()].to_string()
    }
}

/// Directory part used for `MkdirAll`/temp files. Matches Go
/// `filepath.Dir` for the canonical, separator-terminated inputs
/// this module sees.
fn file_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        Some(_) => PathBuf::from("."),
        None => path.to_path_buf(),
    }
}

fn path_is_blank(path: &Path) -> bool {
    path.to_string_lossy().trim().is_empty()
}

/// Sidecar path for a session transcript (Go `branchMetaPath`).
fn branch_meta_path(session_path: &Path) -> PathBuf {
    naming::session_meta(session_path)
}

/// Lock path guarding one session's metadata (Go
/// `sessionMetaLockPath`): the first 8 bytes of the SHA-256 of the
/// canonical save path, next to the canonical file itself.
fn session_meta_lock_path(canonical: &str) -> PathBuf {
    let digest: [u8; 32] = Sha256::digest(canonical.as_bytes()).into();
    let hex = digest_string(&digest);
    file_dir(Path::new(canonical)).join(format!(".{}.meta.lock", &hex[..16]))
}

/// Go save.go `LockSessionMetaPath`.
pub(crate) fn lock_session_meta_path(
    session_path: &Path,
) -> Result<FileLockGuard, BranchMetaError> {
    if path_is_blank(session_path) {
        return Err(BranchMetaError::EmptyPath);
    }
    let canonical = canonical_session_save_path(&session_path.to_string_lossy());
    mkdir_all_mode(&file_dir(Path::new(&canonical)), 0o755)
        .map_err(BranchMetaError::CreateMetaDir)?;
    filelock::acquire(&session_meta_lock_path(&canonical), SESSION_META_LOCK_WAIT)
        .map_err(BranchMetaError::LockSessionMeta)
}

// ---------------------------------------------------------------------------
// Load / sanitize
// ---------------------------------------------------------------------------

/// Go branch.go `LoadBranchMeta`: tolerant sidecar read. Missing
/// or all-NUL files read as absent; a decode failure retries through the
/// UTF-16/GBK fallback decoder before failing with the first error.
pub fn load_branch_meta(session_path: &Path) -> Result<Option<BranchMeta>, BranchMetaError> {
    #[cfg(test)]
    {
        let hook = *BRANCH_META_READ_HOOK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(hook) = hook {
            hook(session_path);
        }
    }
    let meta_path = branch_meta_path(session_path);
    if meta_path.as_os_str().is_empty() {
        return Ok(None);
    }
    let bytes = match std::fs::read(&meta_path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(BranchMetaError::Io(e)),
    };
    if !bytes.is_empty() && bytes.iter().all(|&b| b == 0) {
        return Ok(None);
    }
    let mut meta: BranchMeta = match serde_json::from_slice(&bytes) {
        Ok(meta) => meta,
        Err(first) => match serde_json::from_slice(decode_to_utf8(&bytes).as_ref()) {
            Ok(meta) => meta,
            Err(_) => {
                return Err(BranchMetaError::DecodeBranchMeta {
                    meta_path: meta_path.to_string_lossy().into_owned(),
                    source: first,
                });
            }
        },
    };
    if meta.id.is_empty() {
        meta.id = branch_id(&session_path.to_string_lossy());
    }
    sanitize_display_fields(&mut meta);
    Ok(Some(meta))
}

/// Go branch.go `sanitizeDisplayFields`.
fn sanitize_display_fields(meta: &mut BranchMeta) {
    meta.topic_title = sanitize_stored_display_text(&meta.topic_title);
    meta.custom_title = sanitize_stored_display_text(&meta.custom_title);
    meta.preview = sanitize_stored_display_text(&meta.preview);
}

/// Go branch.go `sanitizeStoredDisplayText`.
fn sanitize_stored_display_text(value: &str) -> String {
    if value.trim().is_empty() {
        value.trim().to_string()
    } else {
        user_preview_text(value)
    }
}

/// Go branch.go `loadBranchMetaRetry`: four tolerant reads around
/// Windows `ReplaceFile` tearing (20/50/100ms backoffs).
pub(crate) fn load_branch_meta_retry(
    session_path: &Path,
) -> Result<Option<BranchMeta>, BranchMetaError> {
    for backoff in BRANCH_META_READ_BACKOFFS {
        match load_branch_meta(session_path) {
            Ok(meta) => return Ok(meta),
            Err(_) => std::thread::sleep(backoff),
        }
    }
    load_branch_meta(session_path)
}

/// Go branch.go `preserveBranchMetaPersistence`: the revision
/// ledger only moves forward; an equal revision backfills blanks
/// instead of clobbering what the previous writer recorded.
fn preserve_branch_meta_persistence(next: &mut BranchMeta, existing: &BranchMeta) {
    if existing.revision > next.revision {
        next.revision = existing.revision;
        next.content_digest = existing.content_digest.clone();
        next.writer_id = existing.writer_id.clone();
        return;
    }
    if existing.revision == next.revision {
        if next.content_digest.trim().is_empty() {
            next.content_digest = existing.content_digest.clone();
        }
        if next.writer_id.trim().is_empty() {
            next.writer_id = existing.writer_id.clone();
        }
    }
}

// ---------------------------------------------------------------------------
// Save / update
// ---------------------------------------------------------------------------

/// Go branch.go `saveBranchMetaAfter`.
pub(crate) fn save_branch_meta_after(
    session_path: &Path,
    meta: &mut BranchMeta,
    touch_updated: bool,
    persisted: Option<&BranchMeta>,
) -> Result<(), BranchMetaError> {
    let meta_path = branch_meta_path(session_path);
    if meta_path.as_os_str().is_empty() {
        return Err(BranchMetaError::EmptyPath);
    }
    let now = now_utc();
    if meta.id.is_empty() {
        meta.id = branch_id(&session_path.to_string_lossy());
    }
    if meta.created_at.is_zero() {
        meta.created_at = now;
    }
    if touch_updated || meta.updated_at.is_zero() {
        meta.updated_at = now;
    }
    if let Some(existing) = persisted {
        preserve_branch_meta_persistence(meta, existing);
    }
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut json = escape_html(&json);
    json.push('\n');
    let dir = file_dir(&meta_path);
    mkdir_all_mode(&dir, 0o755)?;
    let (mut file, tmp_path) = create_temp_file(&dir, ".branch.", ".tmp")?;
    if let Err(e) = file.write_all(json.as_bytes()) {
        drop(file);
        let _ = std::fs::remove_file(&tmp_path);
        return Err(BranchMetaError::Io(e));
    }
    drop(file);
    if let Err(e) = replace_file(&tmp_path, &meta_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(BranchMetaError::Io(e));
    }
    Ok(())
}

/// Go branch.go `saveBranchMeta` (the load-persisted-then-save
/// half, without taking the metadata lock; callers hold it).
fn save_branch_meta_unlocked(
    session_path: &Path,
    meta: &mut BranchMeta,
    touch_updated: bool,
) -> Result<(), BranchMetaError> {
    let persisted = load_branch_meta(session_path)?;
    save_branch_meta_after(session_path, meta, touch_updated, persisted.as_ref())
}

/// Go branch.go `UpdateBranchMeta`: take the metadata lock, ensure
/// the sidecar exists, apply `update`, persist with the previous
/// snapshot as the persistence baseline.
pub fn update_branch_meta(
    session_path: &Path,
    touch_updated: bool,
    update: impl FnOnce(&mut BranchMeta) -> Result<(), BranchMetaError>,
) -> Result<BranchMeta, BranchMetaError> {
    let _lock = lock_session_meta_path(session_path)?;
    let mut meta = ensure_branch_meta_unlocked(session_path)?;
    let persisted = meta.clone();
    update(&mut meta)?;
    save_branch_meta_after(session_path, &mut meta, touch_updated, Some(&persisted))?;
    Ok(meta)
}

/// Go branch.go `EnsureBranchMeta`.
pub fn ensure_branch_meta(session_path: &Path) -> Result<BranchMeta, BranchMetaError> {
    update_branch_meta(session_path, false, |_| Ok(()))
}

/// Go branch.go `ensureBranchMetaUnlocked`: load, or synthesize a
/// fresh sidecar stamped from the transcript's mtime.
pub(crate) fn ensure_branch_meta_unlocked(
    session_path: &Path,
) -> Result<BranchMeta, BranchMetaError> {
    if path_is_blank(session_path) {
        return Err(BranchMetaError::EmptyPath);
    }
    if let Some(meta) = load_branch_meta(session_path)? {
        return Ok(meta);
    }
    let mut when = now_utc();
    if let Ok(modified) = std::fs::metadata(session_path).and_then(|info| info.modified()) {
        when = CreatedAt::from_datetime(chrono::DateTime::<Utc>::from(modified).fixed_offset());
    }
    let mut meta = BranchMeta {
        id: branch_id(&session_path.to_string_lossy()),
        created_at: when,
        updated_at: when,
        ..BranchMeta::default()
    };
    save_branch_meta_unlocked(session_path, &mut meta, false)?;
    Ok(meta)
}

// ---------------------------------------------------------------------------
// Save family and mutators
// ---------------------------------------------------------------------------

/// Go branch.go `SaveBranchMeta`.
pub fn save_branch_meta(session_path: &Path, mut meta: BranchMeta) -> Result<(), BranchMetaError> {
    update_branch_meta(session_path, true, move |current| {
        preserve_branch_meta_persistence(&mut meta, current);
        *current = meta;
        Ok(())
    })?;
    Ok(())
}

/// Go branch.go `SaveBranchMetaKeepInFlightTurn`: a rewrite that
/// does not carry its own marker keeps the persisted one.
pub fn save_branch_meta_keep_in_flight_turn(
    session_path: &Path,
    mut meta: BranchMeta,
) -> Result<(), BranchMetaError> {
    update_branch_meta(session_path, true, move |current| {
        if meta.in_flight_turn.is_none() {
            meta.in_flight_turn = current.in_flight_turn.clone();
        }
        preserve_branch_meta_persistence(&mut meta, current);
        *current = meta;
        Ok(())
    })?;
    Ok(())
}

/// Go branch.go `SaveBranchMetaPreserveUpdated`.
pub fn save_branch_meta_preserve_updated(
    session_path: &Path,
    meta: BranchMeta,
) -> Result<(), BranchMetaError> {
    update_branch_meta(session_path, false, move |current| {
        let mut next = meta;
        preserve_branch_meta_persistence(&mut next, current);
        *current = next;
        Ok(())
    })?;
    Ok(())
}

/// Go branch.go `SaveBranchMetaPreserveUpdatedLocked` — the caller
/// already holds the metadata lock.
pub fn save_branch_meta_preserve_updated_locked(
    session_path: &Path,
    meta: &mut BranchMeta,
) -> Result<(), BranchMetaError> {
    save_branch_meta_unlocked(session_path, meta, false)
}

/// Go branch.go `SetSessionArchived`.
pub fn set_session_archived(session_path: &Path, archived: bool) -> Result<(), BranchMetaError> {
    update_branch_meta(session_path, !archived, |meta| {
        meta.archived = archived;
        meta.auto_archived_at = CreatedAt::zero();
        if archived {
            meta.pinned = false;
        }
        Ok(())
    })?;
    Ok(())
}

/// Go branch.go `SetSessionRank`.
pub fn set_session_rank(session_path: &Path, rank: f64) -> Result<(), BranchMetaError> {
    if rank < 0.0 || rank.is_nan() || rank.is_infinite() {
        return Err(BranchMetaError::RankInvalid(go_v_f64(rank)));
    }
    update_branch_meta(session_path, false, |meta| {
        meta.manual_rank = rank;
        Ok(())
    })?;
    Ok(())
}

/// Go branch.go `SetSessionPinned`.
pub fn set_session_pinned(session_path: &Path, pinned: bool) -> Result<(), BranchMetaError> {
    update_branch_meta(session_path, false, |meta| {
        meta.pinned = pinned;
        Ok(())
    })?;
    Ok(())
}

/// Go branch.go `RenameSession`.
pub fn rename_session(session_path: &Path, title: &str) -> Result<(), BranchMetaError> {
    if path_is_blank(session_path) {
        return Err(BranchMetaError::EmptyPath);
    }
    let _lock = lock_session_meta_path(session_path)?;
    let mut meta = ensure_branch_meta_unlocked(session_path)?;
    meta.custom_title = title.trim().to_string();
    save_branch_meta_unlocked(session_path, &mut meta, false)
}

/// Go branch.go `LoadSessionModel`.
pub fn load_session_model(session_path: &Path) -> Result<Option<String>, BranchMetaError> {
    let Some(meta) = load_branch_meta(session_path)? else {
        return Ok(None);
    };
    let model = meta.model.trim();
    if model.is_empty() {
        Ok(None)
    } else {
        Ok(Some(model.to_string()))
    }
}

/// Go branch.go `SetBranchModelPreserveUpdated`.
pub fn set_branch_model_preserve_updated(
    session_path: &Path,
    model: &str,
) -> Result<(), BranchMetaError> {
    if path_is_blank(session_path) {
        return Err(BranchMetaError::EmptyPath);
    }
    let _lock = lock_session_meta_path(session_path)?;
    let mut meta = ensure_branch_meta_unlocked(session_path)?;
    meta.model = model.trim().to_string();
    save_branch_meta_unlocked(session_path, &mut meta, false)
}

/// Go branch.go `UpdateSessionMeta`.
pub fn update_session_meta(
    session_path: &Path,
    model: &str,
    preview: String,
    turns: i64,
    mark_activity: bool,
) -> Result<(), BranchMetaError> {
    if path_is_blank(session_path) {
        return Err(BranchMetaError::EmptyPath);
    }
    let _lock = lock_session_meta_path(session_path)?;
    let mut meta = ensure_branch_meta_unlocked(session_path)?;
    let persisted = meta.clone();
    let trimmed_model = model.trim();
    if !trimmed_model.is_empty() {
        meta.model = trimmed_model.to_string();
    }
    if meta.superseded && mark_activity && turns > meta.turns {
        meta.superseded = false;
    }
    meta.preview = preview;
    meta.turns = turns;
    meta.schema_version = BRANCH_META_COUNTS_VERSION;
    save_branch_meta_after(session_path, &mut meta, mark_activity, Some(&persisted))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// In-flight turn
// ---------------------------------------------------------------------------

/// Go branch.go `BeginSessionInFlightTurn`.
pub fn begin_session_in_flight_turn(
    session_path: &Path,
    start_message_index: i64,
    preserve_user: bool,
) -> Result<InFlightTurnMeta, BranchMetaError> {
    if path_is_blank(session_path) {
        return Err(BranchMetaError::EmptyPath);
    }
    let _lock = lock_session_meta_path(session_path)?;
    let mut meta = ensure_branch_meta_unlocked(session_path)?;
    let marker = InFlightTurnMeta {
        id: format!(
            "{}-{}-{}",
            session_writer_id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            IN_FLIGHT_TURN_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1
        ),
        start_message_index: start_message_index.max(0),
        preserve_user,
        started_at: now_utc(),
        start_revision: meta.revision,
        start_digest: meta.content_digest.trim().to_string(),
        commit_digest: String::new(),
    };
    meta.in_flight_turn = Some(marker.clone());
    save_branch_meta_unlocked(session_path, &mut meta, false)?;
    Ok(marker)
}

/// Go branch.go `MarkSessionInFlightTurn`.
pub fn mark_session_in_flight_turn(
    session_path: &Path,
    start_message_index: i64,
    preserve_user: bool,
) -> Result<(), BranchMetaError> {
    begin_session_in_flight_turn(session_path, start_message_index, preserve_user).map(|_| ())
}

/// Go branch.go `SetSessionInFlightTurn`: verbatim marker rewrite.
pub fn set_session_in_flight_turn(
    session_path: &Path,
    mut marker: InFlightTurnMeta,
) -> Result<(), BranchMetaError> {
    if path_is_blank(session_path) {
        return Err(BranchMetaError::EmptyPath);
    }
    let _lock = lock_session_meta_path(session_path)?;
    let mut meta = ensure_branch_meta_unlocked(session_path)?;
    marker.start_message_index = marker.start_message_index.max(0);
    if marker.started_at.is_zero() {
        marker.started_at = now_utc();
    }
    meta.in_flight_turn = Some(marker);
    save_branch_meta_unlocked(session_path, &mut meta, false)
}

/// Go branch.go `sameInFlightTurn`.
fn same_in_flight_turn(a: &InFlightTurnMeta, b: &InFlightTurnMeta) -> bool {
    a.id == b.id
        && a.start_message_index == b.start_message_index
        && a.preserve_user == b.preserve_user
        && a.started_at == b.started_at
        && a.start_revision == b.start_revision
        && a.start_digest == b.start_digest
        && a.commit_digest == b.commit_digest
}

/// Go branch.go: a zero-value expected marker matches any in-flight
/// turn; a marker carrying any identity field must match exactly.
fn in_flight_turn_has_identity(marker: &InFlightTurnMeta) -> bool {
    marker.start_message_index != 0
        || marker.preserve_user
        || !marker.started_at.is_zero()
        || marker.start_revision != 0
        || !marker.start_digest.is_empty()
}

/// Go branch.go `ClearSessionInFlightTurnIfMatch`.
pub fn clear_session_in_flight_turn_if_match(
    session_path: &Path,
    expected: &InFlightTurnMeta,
) -> Result<bool, BranchMetaError> {
    let _lock = lock_session_meta_path(session_path)?;
    let mut meta = match load_branch_meta(session_path)? {
        Some(meta) => meta,
        None => return Ok(false),
    };
    let Some(persisted) = meta.in_flight_turn.clone() else {
        return Ok(false);
    };
    if !expected.id.is_empty() {
        if persisted.id != expected.id {
            return Ok(false);
        }
    } else if in_flight_turn_has_identity(expected) && !same_in_flight_turn(&persisted, expected) {
        return Ok(false);
    }
    meta.in_flight_turn = None;
    save_branch_meta_unlocked(session_path, &mut meta, false)?;
    Ok(true)
}

/// Go branch.go `PrepareSessionInFlightTurnCommit`.
pub fn prepare_session_in_flight_turn_commit(
    session_path: &Path,
    expected: &InFlightTurnMeta,
    digest: &str,
) -> Result<Option<InFlightTurnMeta>, BranchMetaError> {
    let trimmed = digest.trim();
    if path_is_blank(session_path) || expected.id.is_empty() || trimmed.is_empty() {
        return Ok(None);
    }
    let _lock = lock_session_meta_path(session_path)?;
    let mut meta = match load_branch_meta(session_path)? {
        Some(meta) => meta,
        None => return Ok(None),
    };
    let Some(mut updated) = meta.in_flight_turn.clone() else {
        return Ok(None);
    };
    if updated.id != expected.id {
        return Ok(None);
    }
    updated.commit_digest = trimmed.to_string();
    meta.in_flight_turn = Some(updated.clone());
    save_branch_meta_unlocked(session_path, &mut meta, false)?;
    Ok(Some(updated))
}

// ---------------------------------------------------------------------------
// Revision ledger (Go save.go)
// ---------------------------------------------------------------------------

/// Go save.go `sessionContentRevision`.
pub(crate) fn session_content_revision(
    session_path: &Path,
) -> Result<(i64, String), BranchMetaError> {
    match load_branch_meta_retry(session_path)? {
        Some(meta) => Ok((meta.revision, meta.content_digest.trim().to_string())),
        None => Ok((0, String::new())),
    }
}

/// Go save.go `recordSessionContentRevision`: the read-modify-write
/// revision ledger, serialized by the per-session metadata lock.
pub(crate) fn record_session_content_revision(
    session_path: &Path,
    digest: &[u8; 32],
    base_revision: i64,
) -> Result<i64, BranchMetaError> {
    let _lock = lock_session_meta_path(session_path)?;
    let read = load_branch_meta_retry(session_path)?;
    let (mut meta, persisted) = match read {
        Some(existing) => (existing.clone(), Some(existing)),
        None => (
            BranchMeta {
                id: branch_id(&session_path.to_string_lossy()),
                ..BranchMeta::default()
            },
            None,
        ),
    };
    if meta.revision < base_revision {
        meta.revision = base_revision;
    }
    meta.revision += 1;
    meta.content_digest = digest_string(digest);
    meta.writer_id = session_writer_id().to_string();
    save_branch_meta_after(session_path, &mut meta, false, persisted.as_ref())?;
    Ok(meta.revision)
}

// ---------------------------------------------------------------------------
// Float formatting
// ---------------------------------------------------------------------------

/// Formats a float the way Go's `fmt` `%v` verb does
/// (strconv 'g' with shortest round-trip): decimal notation for
/// exponents in `[-4, 6)`, scientific with a signed exponent
/// (two digits minimum) otherwise. Verified against
/// `.parity-gocopy/cmd/fmtprobe`.
pub fn go_v_f64(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "+Inf".to_string()
        } else {
            "-Inf".to_string()
        };
    }
    let sci = format!("{value:e}");
    let (mantissa, exponent) = sci.split_once('e').unwrap_or((sci.as_str(), "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    if !(-4..6).contains(&exponent) {
        let (sign, digits) = if exponent < 0 {
            ("-", exponent.unsigned_abs())
        } else {
            ("+", exponent as u32)
        };
        format!("{mantissa}e{sign}{digits:02}")
    } else {
        format!("{value}")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::eventlog::test_support::tempdir;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    /// Creates a transcript placeholder the sidecar functions accept.
    fn session_file(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"{}\n").expect("write session transcript");
        path
    }

    #[test]
    fn path_base_matches_go_filepathlite() {
        assert_eq!(path_base(""), ".");
        assert_eq!(path_base("C:\\"), "\\");
        assert_eq!(path_base("C:"), "\\");
        assert_eq!(path_base(r"C:\a\b\c.txt"), "c.txt");
        assert_eq!(path_base("C:\\a\\b\\"), "b");
        assert_eq!(path_base(r"\\server\share\file.txt"), "file.txt");
        assert_eq!(path_base(r"\\.\UNC\server\share\x"), "x");
        assert_eq!(path_base("a/b/c"), "c");
        assert_eq!(path_base("/"), "\\");
        assert_eq!(path_base("C:/a/b/c.txt"), "c.txt");
    }

    #[test]
    fn path_ext_matches_go_filepathlite() {
        assert_eq!(path_ext("a.tar.gz"), ".gz");
        assert_eq!(path_ext(".hidden"), ".hidden");
        assert_eq!(path_ext("a/.hidden"), ".hidden");
        assert_eq!(path_ext("a.b/c"), "");
        assert_eq!(path_ext(r"C:\dir\file.TXT"), ".TXT");
        assert_eq!(path_ext(""), "");
        assert_eq!(path_ext("noext"), "");
    }

    #[test]
    fn branch_id_strips_extension() {
        assert_eq!(branch_id(""), "");
        assert_eq!(
            branch_id(r"C:\sessions\20261117-120000.000000000-model.jsonl"),
            "20261117-120000.000000000-model"
        );
        assert_eq!(branch_id(".hidden"), "");
        assert_eq!(branch_id("a/b/c.txt"), "c");
        assert_eq!(branch_id("C:\\a\\b\\"), "b");
        assert_eq!(branch_id("plain"), "plain");
    }

    #[test]
    fn go_v_f64_matches_go_percent_v() {
        assert_eq!(go_v_f64(999_999.0), "999999");
        assert_eq!(go_v_f64(1_000_000.0), "1e+06");
        assert_eq!(go_v_f64(1_234_567.0), "1.234567e+06");
        assert_eq!(go_v_f64(0.0001), "0.0001");
        assert_eq!(go_v_f64(0.00001), "1e-05");
        assert_eq!(go_v_f64(9_999_995.0), "9.999995e+06");
        assert_eq!(go_v_f64(-1_500_000.0), "-1.5e+06");
        assert_eq!(go_v_f64(999_999.999_999_999_9), "999999.9999999999");
        assert_eq!(go_v_f64(0.000_099_999_999_99), "9.999999999e-05");
        assert_eq!(go_v_f64(123.456_789_012_345_68), "123.45678901234568");
        assert_eq!(go_v_f64(1e100), "1e+100");
        assert_eq!(go_v_f64(5e-324), "5e-324");
        assert_eq!(
            go_v_f64(1.797_693_134_862_315_7e308),
            "1.7976931348623157e+308"
        );
        assert_eq!(go_v_f64(-0.0), "-0");
        assert_eq!(go_v_f64(0.0), "0");
        assert_eq!(go_v_f64(1.5), "1.5");
        assert_eq!(go_v_f64(100.0), "100");
        assert_eq!(go_v_f64(f64::NAN), "NaN");
        assert_eq!(go_v_f64(f64::INFINITY), "+Inf");
        assert_eq!(go_v_f64(f64::NEG_INFINITY), "-Inf");
    }

    #[test]
    fn rank_rejects_invalid_values() {
        let dir = tempdir();
        let session = session_file(dir.path(), "rank.jsonl");
        let err = set_session_rank(&session, -1.5).unwrap_err();
        assert_eq!(
            err.to_string(),
            "a session rank is a finite number from 0, not -1.5"
        );
        assert_eq!(
            set_session_rank(&session, f64::NAN)
                .unwrap_err()
                .to_string(),
            "a session rank is a finite number from 0, not NaN"
        );
        assert_eq!(
            set_session_rank(&session, f64::INFINITY)
                .unwrap_err()
                .to_string(),
            "a session rank is a finite number from 0, not +Inf"
        );
    }

    #[test]
    fn save_round_trips_through_escaped_json() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "20261117-120000.000000000-model.jsonl");
        let meta = BranchMeta {
            name: "<b>&name</b>".to_string(),
            ..BranchMeta::default()
        };
        save_branch_meta(&session, meta)?;
        let raw = fs::read_to_string(branch_meta_path(&session))?;
        assert!(raw.starts_with("{\n  \""));
        assert!(raw.ends_with("}\n"));
        assert!(raw.contains("\\u003c"));
        assert!(raw.contains("\\u003e"));
        assert!(raw.contains("\\u0026"));
        assert!(!raw.contains('<'));
        assert!(!raw.contains('&'));
        let loaded = load_branch_meta(&session)?.expect("meta loads");
        assert_eq!(loaded.name, "<b>&name</b>");
        Ok(())
    }

    #[test]
    fn fresh_meta_has_exactly_id_and_timestamps() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "fresh.jsonl");
        let meta = ensure_branch_meta(&session)?;
        assert!(!meta.id.is_empty());
        assert!(!meta.created_at.is_zero());
        assert!(!meta.updated_at.is_zero());
        assert_eq!(meta.created_at, meta.updated_at);
        let raw = fs::read_to_string(branch_meta_path(&session))?;
        let value: serde_json::Value = serde_json::from_str(&raw).expect("valid json");
        let object = value.as_object().expect("object");
        assert_eq!(object.len(), 3);
        assert!(object.contains_key("id"));
        assert!(object.contains_key("created_at"));
        assert!(object.contains_key("updated_at"));
        Ok(())
    }

    #[test]
    fn meta_key_order_matches_go() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "order.jsonl");
        let meta = BranchMeta {
            name: "ordered".to_string(),
            manual_rank: 1.5,
            in_flight_turn: Some(InFlightTurnMeta::default()),
            adopted: Some(AdoptRecord::default()),
            ..BranchMeta::default()
        };
        save_branch_meta(&session, meta)?;
        let raw = fs::read_to_string(branch_meta_path(&session))?;
        let position = |key: &str| raw.find(&format!("\"{key}\":")).unwrap_or(usize::MAX);
        assert_ne!(position("adopted"), usize::MAX);
        assert!(position("id") < position("name"));
        assert!(position("name") < position("created_at"));
        assert!(position("created_at") < position("updated_at"));
        assert!(position("updated_at") < position("manual_rank"));
        assert!(position("manual_rank") < position("in_flight_turn"));
        assert!(position("in_flight_turn") < position("adopted"));
        Ok(())
    }

    #[test]
    fn load_tolerates_missing_and_blank_inputs() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "missing.jsonl");
        assert!(load_branch_meta(&session)?.is_none());
        assert!(load_branch_meta(Path::new(""))?.is_none());
        fs::write(branch_meta_path(&session), [0u8, 0, 0])?;
        assert!(load_branch_meta(&session)?.is_none());
        Ok(())
    }

    #[test]
    fn load_falls_back_to_utf16() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "utf16.jsonl");
        let json = "{\"id\":\"utf16-id\",\"created_at\":\"2026-01-02T03:04:05Z\",\"updated_at\":\"2026-01-02T03:04:05Z\"}";
        let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
        for unit in json.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(branch_meta_path(&session), &bytes)?;
        let meta = load_branch_meta(&session)?.expect("utf16 meta loads");
        assert_eq!(meta.id, "utf16-id");
        assert!(!meta.created_at.is_zero());
        Ok(())
    }

    #[test]
    fn load_reports_decode_errors_with_path() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "broken.jsonl");
        fs::write(branch_meta_path(&session), b"\x80\x81 not json")?;
        let err = load_branch_meta(&session).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("decode branch meta"), "{message}");
        assert!(message.contains("broken.jsonl.meta"), "{message}");
        Ok(())
    }

    #[test]
    fn preserve_blocks_revision_regression() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "preserve.jsonl");
        update_branch_meta(&session, false, |meta| {
            meta.revision = 5;
            meta.content_digest = "digest-existing".to_string();
            meta.writer_id = "writer-one".to_string();
            Ok(())
        })?;
        let loaded = load_branch_meta(&session)?.expect("meta");
        let mut regressed = loaded.clone();
        regressed.revision = 3;
        regressed.content_digest = "digest-regressed".to_string();
        regressed.writer_id = "writer-two".to_string();
        save_branch_meta_preserve_updated(&session, regressed)?;
        let after = load_branch_meta(&session)?.expect("meta");
        assert_eq!(after.revision, 5);
        assert_eq!(after.content_digest, "digest-existing");
        assert_eq!(after.writer_id, "writer-one");
        Ok(())
    }

    #[test]
    fn equal_revision_backfills_blanks() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "backfill.jsonl");
        update_branch_meta(&session, false, |meta| {
            meta.revision = 2;
            meta.content_digest = "digest-old".to_string();
            meta.writer_id = "writer-old".to_string();
            Ok(())
        })?;
        let loaded = load_branch_meta(&session)?.expect("meta");
        let mut blank = loaded.clone();
        blank.content_digest = String::new();
        blank.writer_id = String::new();
        save_branch_meta_preserve_updated(&session, blank)?;
        let after = load_branch_meta(&session)?.expect("meta");
        assert_eq!(after.revision, 2);
        assert_eq!(after.content_digest, "digest-old");
        assert_eq!(after.writer_id, "writer-old");
        Ok(())
    }

    #[test]
    fn update_session_meta_stamps_counts() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "counts.jsonl");
        update_branch_meta(&session, false, |meta| {
            meta.model = "model-old".to_string();
            meta.turns = 2;
            meta.superseded = true;
            Ok(())
        })?;
        update_session_meta(&session, "  model-x  ", "preview one".to_string(), 5, true)?;
        let meta = load_branch_meta(&session)?.expect("meta");
        assert_eq!(meta.model, "model-x");
        assert_eq!(meta.preview, "preview one");
        assert_eq!(meta.turns, 5);
        assert_eq!(meta.schema_version, BRANCH_META_COUNTS_VERSION);
        assert!(!meta.superseded);
        // An empty model keeps the persisted one.
        update_session_meta(&session, "   ", "preview two".to_string(), 6, true)?;
        let meta = load_branch_meta(&session)?.expect("meta");
        assert_eq!(meta.model, "model-x");
        assert_eq!(meta.turns, 6);
        Ok(())
    }

    #[test]
    fn superseded_clears_only_on_activity() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "superseded.jsonl");
        update_branch_meta(&session, false, |meta| {
            meta.superseded = true;
            meta.turns = 2;
            Ok(())
        })?;
        update_session_meta(&session, "model-x", "p".to_string(), 5, false)?;
        let meta = load_branch_meta(&session)?.expect("meta");
        assert!(meta.superseded);
        update_session_meta(&session, "model-x", "p".to_string(), 5, true)?;
        let meta = load_branch_meta(&session)?.expect("meta");
        assert!(meta.superseded);
        update_session_meta(&session, "model-x", "p".to_string(), 6, true)?;
        let meta = load_branch_meta(&session)?.expect("meta");
        assert!(!meta.superseded);
        Ok(())
    }

    #[test]
    fn load_session_model_trims() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "model.jsonl");
        assert_eq!(load_session_model(&session)?, None);
        update_branch_meta(&session, false, |meta| {
            meta.model = "  model-y  ".to_string();
            Ok(())
        })?;
        assert_eq!(load_session_model(&session)?, Some("model-y".to_string()));
        Ok(())
    }

    #[test]
    fn in_flight_turn_lifecycle() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "inflight.jsonl");
        update_branch_meta(&session, false, |meta| {
            meta.revision = 7;
            meta.content_digest = "  hexdigest  ".to_string();
            Ok(())
        })?;
        let marker = begin_session_in_flight_turn(&session, 3, true)?;
        assert!(marker.id.starts_with(session_writer_id()));
        assert_eq!(marker.start_message_index, 3);
        assert!(marker.preserve_user);
        assert!(!marker.started_at.is_zero());
        assert_eq!(marker.start_revision, 7);
        assert_eq!(marker.start_digest, "hexdigest");
        assert_eq!(marker.commit_digest, "");
        let meta = load_branch_meta(&session)?.expect("meta");
        assert_eq!(
            serde_json::to_string(&meta.in_flight_turn).unwrap(),
            serde_json::to_string(&marker).unwrap()
        );
        // Negative indexes clamp to zero.
        let clamped = begin_session_in_flight_turn(&session, -5, false)?;
        assert_eq!(clamped.start_message_index, 0);
        // Prepare refuses a foreign marker id and a blank digest.
        let mut foreign = marker.clone();
        foreign.id = "other-writer".to_string();
        assert!(prepare_session_in_flight_turn_commit(&session, &foreign, "d")?.is_none());
        assert!(prepare_session_in_flight_turn_commit(&session, &clamped, "   ")?.is_none());
        // The owned marker commits, with the digest trimmed.
        let updated =
            prepare_session_in_flight_turn_commit(&session, &clamped, "  commit-digest  ")?
                .expect("prepared");
        assert_eq!(updated.commit_digest, "commit-digest");
        let meta = load_branch_meta(&session)?.expect("meta");
        assert_eq!(
            serde_json::to_string(&meta.in_flight_turn).unwrap(),
            serde_json::to_string(&updated).unwrap()
        );
        // Clear matches only the owning marker.
        let mut wrong = marker.clone();
        wrong.id = "not-it".to_string();
        assert!(!clear_session_in_flight_turn_if_match(&session, &wrong)?);
        assert!(clear_session_in_flight_turn_if_match(&session, &updated)?);
        assert!(!clear_session_in_flight_turn_if_match(&session, &updated)?);
        let meta = load_branch_meta(&session)?.expect("meta");
        assert!(meta.in_flight_turn.is_none());
        Ok(())
    }

    #[test]
    fn in_flight_legacy_marker_clears_by_field_match() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "legacy.jsonl");
        let legacy = InFlightTurnMeta {
            start_message_index: 4,
            preserve_user: true,
            ..InFlightTurnMeta::default()
        };
        set_session_in_flight_turn(&session, legacy)?;
        let persisted = load_branch_meta(&session)?
            .expect("meta")
            .in_flight_turn
            .expect("marker");
        assert!(persisted.id.is_empty());
        assert!(!persisted.started_at.is_zero());
        let mut mismatch = persisted.clone();
        mismatch.start_message_index = 99;
        assert!(!clear_session_in_flight_turn_if_match(&session, &mismatch)?);
        assert!(clear_session_in_flight_turn_if_match(&session, &persisted)?);
        assert!(
            load_branch_meta(&session)?
                .expect("meta")
                .in_flight_turn
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn zero_value_expected_clears_any_marker() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "zero.jsonl");
        begin_session_in_flight_turn(&session, 1, false)?;
        assert!(clear_session_in_flight_turn_if_match(
            &session,
            &InFlightTurnMeta::default()
        )?);
        Ok(())
    }

    #[test]
    fn set_session_in_flight_turn_clamps_and_stamps() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "setmarker.jsonl");
        set_session_in_flight_turn(
            &session,
            InFlightTurnMeta {
                id: "custom-id".to_string(),
                start_message_index: -2,
                ..InFlightTurnMeta::default()
            },
        )?;
        let marker = load_branch_meta(&session)?
            .expect("meta")
            .in_flight_turn
            .expect("marker");
        assert_eq!(marker.id, "custom-id");
        assert_eq!(marker.start_message_index, 0);
        assert!(!marker.started_at.is_zero());
        Ok(())
    }

    #[test]
    fn revision_ledger_counts_up() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "ledger.jsonl");
        let digest_a: [u8; 32] = [1; 32];
        let digest_b: [u8; 32] = [2; 32];
        let digest_c: [u8; 32] = [3; 32];
        assert_eq!(record_session_content_revision(&session, &digest_a, 0)?, 1);
        assert_eq!(record_session_content_revision(&session, &digest_b, 0)?, 2);
        assert_eq!(
            record_session_content_revision(&session, &digest_c, 10)?,
            11
        );
        let (revision, digest) = session_content_revision(&session)?;
        assert_eq!(revision, 11);
        assert_eq!(digest, digest_string(&digest_c));
        assert_eq!(digest.len(), 64);
        Ok(())
    }

    #[test]
    fn revision_ledger_serializes_concurrent_writers() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = Arc::new(session_file(dir.path(), "threads.jsonl"));
        let digest = [7u8; 32];
        let mut handles = Vec::new();
        for _ in 0..8 {
            let path = Arc::clone(&session);
            handles.push(std::thread::spawn(move || {
                record_session_content_revision(&path, &digest, 0)
            }));
        }
        let mut revisions: Vec<i64> = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .expect("writer thread")
                    .expect("revision recorded")
            })
            .collect();
        revisions.sort_unstable();
        assert_eq!(revisions, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(session_content_revision(&session)?.0, 8);
        Ok(())
    }

    #[test]
    fn archived_clears_pin_and_auto_archive() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "archived.jsonl");
        update_branch_meta(&session, false, |meta| {
            meta.pinned = true;
            meta.auto_archived_at = now_utc();
            Ok(())
        })?;
        set_session_archived(&session, true)?;
        let meta = load_branch_meta(&session)?.expect("meta");
        assert!(meta.archived);
        assert!(!meta.pinned);
        assert!(meta.auto_archived_at.is_zero());
        let raw = fs::read_to_string(branch_meta_path(&session))?;
        assert!(raw.contains("\"archived\": true"));
        assert!(!raw.contains("auto_archived_at"));
        assert!(!raw.contains("\"pinned\""));
        set_session_archived(&session, false)?;
        assert!(!load_branch_meta(&session)?.expect("meta").archived);
        Ok(())
    }

    #[test]
    fn pinned_persists() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "pinned.jsonl");
        set_session_pinned(&session, true)?;
        assert!(load_branch_meta(&session)?.expect("meta").pinned);
        Ok(())
    }

    #[test]
    fn manual_rank_serializes_only_when_set() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "rank-file.jsonl");
        set_session_rank(&session, 1.5)?;
        let raw = fs::read_to_string(branch_meta_path(&session))?;
        assert!(raw.contains("\"manual_rank\": 1.5"));
        assert_eq!(load_branch_meta(&session)?.expect("meta").manual_rank, 1.5);
        set_session_rank(&session, 0.0)?;
        let raw = fs::read_to_string(branch_meta_path(&session))?;
        assert!(!raw.contains("manual_rank"));
        Ok(())
    }

    #[test]
    fn rename_trims_title() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "rename.jsonl");
        rename_session(&session, "  hello world  ")?;
        assert_eq!(
            load_branch_meta(&session)?.expect("meta").custom_title,
            "hello world"
        );
        Ok(())
    }

    #[test]
    fn keep_in_flight_turn_preserves_marker() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "keep.jsonl");
        let marker = begin_session_in_flight_turn(&session, 2, true)?;
        save_branch_meta_keep_in_flight_turn(&session, BranchMeta::default())?;
        let kept = load_branch_meta(&session)?
            .expect("meta")
            .in_flight_turn
            .expect("marker kept");
        assert_eq!(kept.id, marker.id);
        Ok(())
    }

    #[test]
    fn preserve_updated_keeps_updated_at() -> Result<(), BranchMetaError> {
        let dir = tempdir();
        let session = session_file(dir.path(), "preserve-ts.jsonl");
        ensure_branch_meta(&session)?;
        let before = load_branch_meta(&session)?.expect("meta");
        std::thread::sleep(std::time::Duration::from_millis(15));
        let mut edited = before.clone();
        edited.name = "renamed later".to_string();
        save_branch_meta_preserve_updated(&session, edited)?;
        let after = load_branch_meta(&session)?.expect("meta");
        assert_eq!(after.name, "renamed later");
        assert_eq!(after.updated_at, before.updated_at);
        Ok(())
    }

    #[test]
    fn blank_paths_are_rejected() {
        assert!(matches!(
            rename_session(Path::new("  "), "title"),
            Err(BranchMetaError::EmptyPath)
        ));
        assert!(matches!(
            begin_session_in_flight_turn(Path::new(""), 0, true),
            Err(BranchMetaError::EmptyPath)
        ));
        let result: Result<BranchMeta, BranchMetaError> =
            update_branch_meta(Path::new(""), false, |_| Ok(()));
        assert!(matches!(result, Err(BranchMetaError::EmptyPath)));
        assert!(matches!(
            save_branch_meta_preserve_updated(Path::new("   "), BranchMeta::default()),
            Err(BranchMetaError::EmptyPath)
        ));
    }
}
