//! Native import of 1.x sessions-v4 conversations.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use reasonix_contract::provider::Message;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::branch::{
    BRANCH_META_COUNTS_VERSION, BranchMeta, BranchMetaError, ImportSource, branch_id,
    load_branch_meta, save_branch_meta,
};
use super::eventlog::{CreatedAt, probe_session_event_log};
use super::session::{
    LoadError, SaveError, Session, digest_session_messages, is_cleanup_pending,
    load_session_unlocked, session_artifact_exists,
};
use super::session_order::CleanupPendingMeta;
use super::sessionpreview::session_preview_from_messages;
use super::sessionv4::{Session as V4Session, Transcript, V4Error, open, open_in};
use crate::store::naming::{
    is_v4_session_id, session_cleanup_pending, session_display_index, session_event_index,
    session_event_log, session_sidecar_files, session_v4_removed_marker, v4_sessions_root,
};
use reasonix_core::fileutil::{atomic_write_file, mkdir_all_mode};

pub const IMPORT_LINE: &str = "1.x sessions-v4";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    TooLarge,
    UnreadableFormat,
    SchemaUnsupported,
    Permission,
    Corrupt,
    CopyFailed,
    NoProject,
}

impl SkipReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TooLarge => "too_large",
            Self::UnreadableFormat => "unreadable_format",
            Self::SchemaUnsupported => "schema_unsupported",
            Self::Permission => "permission",
            Self::Corrupt => "corrupt",
            Self::CopyFailed => "copy_failed",
            Self::NoProject => "no_project",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkippedSession {
    pub name: String,
    pub path: String,
    pub reason: SkipReason,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImportReport {
    pub imported: usize,
    pub skipped: Vec<SkippedSession>,
}

#[derive(Clone, Debug, Default)]
pub struct V4Origin {
    pub id: String,
    pub cwd: String,
}

#[derive(Clone, Debug, Default)]
pub struct V4Placement {
    pub dir: PathBuf,
    pub known: bool,
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error(transparent)]
    Source(#[from] V4Error),
    #[error(transparent)]
    Load(#[from] LoadError),
    #[error(transparent)]
    Save(#[from] SaveError),
    #[error(transparent)]
    Meta(#[from] BranchMetaError),
    #[error("read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("read legacy source {path}: {source}")]
    SourceRead {
        path: PathBuf,
        reason: SkipReason,
        #[source]
        source: io::Error,
    },
    #[error("session was removed from the 1.x store")]
    Removed,
    #[error("encode or decode session JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl ImportError {
    fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn skip_reason(&self) -> SkipReason {
        match self {
            Self::SourceRead { reason, .. } => *reason,
            _ => classify_write(self),
        }
    }
}

pub fn v4_import_path(sessions_dir: &Path, id: &str) -> PathBuf {
    sessions_dir.join(format!("v4-{id}.jsonl"))
}

pub fn v4_session_for(path: &Path) -> Result<Option<V4Session>, V4Error> {
    let Some(file) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    let Some(stem) = file.strip_suffix(".jsonl") else {
        return Ok(None);
    };
    let Some(id) = stem.strip_prefix("v4-") else {
        return Ok(None);
    };
    let Some(parent) = path.parent() else {
        return Ok(None);
    };
    let root = v4_sessions_root(parent);
    if root.as_os_str().is_empty() || !is_v4_session_id(id) {
        return Ok(None);
    }
    open(&root.join(id)).map(Some)
}

pub fn prepare_session_path(path: &Path) -> Result<(), ImportError> {
    let removed = session_v4_removed_marker(path);
    if !removed.as_os_str().is_empty() {
        let visible = !is_cleanup_pending(&path.to_string_lossy()) && !is_v4_session_removed(path);
        if !visible {
            return Err(ImportError::Removed);
        }
    }
    let Ok(Some(source)) = v4_session_for(path) else {
        return Ok(());
    };
    if !session_artifact_exists(path) {
        return import_v4_as(path, &source, BranchMeta::default());
    }
    let Ok(meta) = load_branch_meta(path) else {
        return Ok(());
    };
    let Some(imported) = meta.as_ref().and_then(|meta| meta.imported_from.as_ref()) else {
        complete_v4_import(path, &source, BranchMeta::default())?;
        return Ok(());
    };
    if imported.log_size == source.log_size {
        return Ok(());
    }
    let Ok(current) = load_session_unlocked(path) else {
        return Ok(());
    };
    let Ok(digest) = digest_session_messages(&current.snapshot()) else {
        return Ok(());
    };
    if hex(&digest) != imported.digest {
        return Ok(());
    }
    let transcript = source.transcript()?;
    current.rewrite(transcript.messages.clone(), "import from 1.x");
    current.save_rewrite(&path.to_string_lossy())?;
    save_import_meta(
        path,
        &source,
        &transcript,
        BranchMeta::default(),
        &transcript.messages,
    )
}

pub fn import_v4_as(path: &Path, source: &V4Session, start: BranchMeta) -> Result<(), ImportError> {
    let transcript = source.transcript()?;
    let session = Session::new("");
    session.replace(transcript.messages.clone());
    match session.save_if_absent(&path.to_string_lossy()) {
        Ok(()) => {}
        Err(SaveError::Exists) => {}
        Err(err) => return Err(err.into()),
    }
    save_import_meta(path, source, &transcript, start, &transcript.messages)
}

pub fn complete_v4_import(
    path: &Path,
    source: &V4Session,
    start: BranchMeta,
) -> Result<bool, ImportError> {
    let meta = match load_branch_meta(path) {
        Ok(meta) => meta,
        Err(_) => return Ok(false),
    };
    if meta
        .as_ref()
        .and_then(|meta| meta.imported_from.as_ref())
        .is_some()
    {
        heal_imported_title(path, source);
        return Ok(false);
    }
    let transcript = source.transcript()?;
    if session_preview_from_messages(&transcript.messages).1 == 0 {
        return Ok(false);
    }
    let content = absorb_interrupted_copy(path, &transcript)?;
    save_import_meta(path, source, &transcript, start, &content)?;
    Ok(true)
}

pub fn absorb_interrupted_copy(
    path: &Path,
    transcript: &Transcript,
) -> Result<Vec<Message>, ImportError> {
    if !session_artifact_exists(path) {
        let session = Session::new("");
        session.replace(transcript.messages.clone());
        match session.save_if_absent(&path.to_string_lossy()) {
            Ok(()) => {}
            Err(SaveError::Exists) => {}
            Err(err) => return Err(err.into()),
        }
        return Ok(transcript.messages.clone());
    }
    let current = load_session_unlocked(path)?;
    let copied = current.snapshot();
    if copied.len() >= transcript.messages.len()
        || !digest_session_messages(&copied)
            .ok()
            .zip(digest_session_messages(&transcript.messages[..copied.len()]).ok())
            .is_some_and(|(have, want)| have == want)
    {
        return Ok(copied);
    }
    current.rewrite(
        transcript.messages.clone(),
        "complete an interrupted import",
    );
    current.save_rewrite(&path.to_string_lossy())?;
    Ok(transcript.messages.clone())
}

fn save_import_meta(
    path: &Path,
    source: &V4Session,
    transcript: &Transcript,
    start: BranchMeta,
    content: &[Message],
) -> Result<(), ImportError> {
    let digest = digest_session_messages(&transcript.messages)?;
    let (preview, turns) = session_preview_from_messages(content);
    let mut meta = load_branch_meta(path).ok().flatten().unwrap_or_default();
    meta.id = branch_id(&path.to_string_lossy());
    meta.created_at = CreatedAt::from_datetime(source.manifest.created_at);
    meta.updated_at = CreatedAt::from_datetime(source.updated_at());
    meta.preview = preview;
    meta.turns = turns;
    meta.schema_version = BRANCH_META_COUNTS_VERSION;
    meta.model = transcript.model_ref.clone();
    let title = if !transcript.title.is_empty() {
        transcript.title.clone()
    } else {
        source
            .cached_listing()
            .map(|listing| listing.title)
            .unwrap_or_default()
    };
    if !title.is_empty() {
        meta.custom_title = title;
    }
    meta.archived |= start.archived;
    meta.pinned |= start.pinned;
    meta.imported_from = Some(ImportSource {
        line: IMPORT_LINE.to_string(),
        log_size: source.log_size,
        digest: hex(&digest),
    });
    save_branch_meta(path, meta)?;
    Ok(())
}

fn heal_imported_title(path: &Path, source: &V4Session) {
    let Ok(Some(mut meta)) = load_branch_meta(path) else {
        return;
    };
    if meta.imported_from.is_none() || !meta.custom_title.is_empty() {
        return;
    }
    if let Some(listing) = source
        .cached_listing()
        .filter(|listing| !listing.title.is_empty())
    {
        meta.custom_title = listing.title;
        let _ = save_branch_meta(path, meta);
    }
}

pub fn v4_copy_begun(path: &Path) -> bool {
    if path.symlink_metadata().is_ok() {
        return true;
    }
    let ignored = [session_event_index(path), session_display_index(path)];
    for artifact in session_sidecar_files(path) {
        if ignored.contains(&artifact) {
            continue;
        }
        if artifact == session_event_log(path) {
            match probe_session_event_log(path) {
                Err(_) => return true,
                Ok(probe) if !probe.native => continue,
                Ok(_) => {}
            }
        }
        if artifact.symlink_metadata().is_ok() {
            return true;
        }
    }
    false
}

pub fn remove_orphan_indexes(path: &Path) -> Result<(), ImportError> {
    for index in [session_event_index(path), session_display_index(path)] {
        match fs::remove_file(&index) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(ImportError::io(index, err)),
        }
    }
    Ok(())
}

pub fn read_v4_cwd(source: &V4Session) -> String {
    let Ok(raw) = source
        .store
        .read(&Path::new(&source.name).join("header.json"))
    else {
        return String::new();
    };
    #[derive(Deserialize)]
    struct Header {
        #[serde(default)]
        cwd: String,
    }
    serde_json::from_slice::<Header>(&raw)
        .map(|header| header.cwd.trim().to_string())
        .unwrap_or_default()
}

pub fn import_v4_from(
    source_root: &Path,
    sessions_dir: &Path,
) -> Result<ImportReport, ImportError> {
    import_v4_from_routed(
        source_root,
        sessions_dir,
        |_| V4Placement {
            dir: PathBuf::new(),
            known: true,
        },
        |_| BranchMeta::default(),
    )
}

pub fn import_v4_from_routed<F, G>(
    source_root: &Path,
    sessions_dir: &Path,
    mut route: F,
    mut imported_as: G,
) -> Result<ImportReport, ImportError>
where
    F: FnMut(V4Origin) -> V4Placement,
    G: FnMut(&str) -> BranchMeta,
{
    let mut entries: Vec<_> = fs::read_dir(source_root)
        .map_err(|err| ImportError::SourceRead {
            path: source_root.to_path_buf(),
            reason: classify_io(err.kind()),
            source: err,
        })?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect();
    entries.sort();
    mkdir_all_mode(sessions_dir, 0o755).map_err(|err| ImportError::SourceRead {
        path: sessions_dir.to_path_buf(),
        reason: classify_io(err.kind()),
        source: err,
    })?;
    let mut report = ImportReport::default();
    for name in entries {
        let source = match open_in(source_root, &name) {
            Ok(source) => source,
            Err(err) if is_not_found(&err) => continue,
            Err(err) => {
                report.skipped.push(SkippedSession {
                    name: name.clone(),
                    path: name,
                    reason: classify_read(&err),
                });
                continue;
            }
        };
        let id = source.manifest.session_id.clone();
        if !is_v4_session_id(&id) {
            report.skipped.push(SkippedSession {
                name: name.clone(),
                path: name,
                reason: SkipReason::UnreadableFormat,
            });
            continue;
        }
        let mut target = sessions_dir.to_path_buf();
        let placed = route(V4Origin {
            id: id.clone(),
            cwd: read_v4_cwd(&source),
        });
        if !placed.dir.as_os_str().is_empty() {
            target = placed.dir;
        } else if !placed.known {
            report.skipped.push(SkippedSession {
                name: name.clone(),
                path: name,
                reason: SkipReason::NoProject,
            });
            continue;
        }
        let path = v4_import_path(&target, &id);
        if !v4_copy_begun(&path)
            && let Err(err) = remove_orphan_indexes(&path)
        {
            tracing::warn!(path = %path.display(), error = %err, "stale import index remains");
        }
        let start = imported_as(&id);
        let result = if session_artifact_exists(&path) {
            complete_v4_import(&path, &source, start).map(usize::from)
        } else {
            let transcript = match source.transcript() {
                Ok(transcript) => transcript,
                Err(err) => {
                    report.skipped.push(SkippedSession {
                        name: name.clone(),
                        path: name,
                        reason: classify_read(&err),
                    });
                    continue;
                }
            };
            if session_preview_from_messages(&transcript.messages).1 == 0 {
                continue;
            }
            mkdir_all_mode(&target, 0o755)
                .map_err(|err| ImportError::io(&target, err))
                .and_then(|()| import_v4_as(&path, &source, start))
                .map(|()| 1)
        };
        match result {
            Ok(imported) => report.imported += imported,
            Err(err) => report.skipped.push(SkippedSession {
                name: name.clone(),
                path: name,
                reason: err.skip_reason(),
            }),
        }
    }
    Ok(report)
}

pub fn is_v4_session_removed(path: &Path) -> bool {
    removed_marker_store(path).is_ok_and(|(store, name)| store.metadata(Path::new(&name)).is_ok())
}

pub fn preserve_v4_removal(path: &Path) -> Result<(), ImportError> {
    let target = session_v4_removed_marker(path);
    if target.as_os_str().is_empty() {
        return Ok(());
    }
    let (store, _) = match removed_marker_store(path) {
        Ok(store) => store,
        Err(ImportError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(());
        }
        Err(err) => return Err(err),
    };
    let pending = session_cleanup_pending(path);
    let Some(name) = pending.file_name() else {
        return Ok(());
    };
    let raw = match store.read(Path::new(name)) {
        Ok(raw) => raw,
        Err(V4Error::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };
    let meta: CleanupPendingMeta = serde_json::from_slice(&raw).map_err(ImportError::Json)?;
    if matches!(meta.operation.as_str(), "remove" | "delete" | "clear") {
        atomic_write_file(&target, &raw, 0o644).map_err(|err| ImportError::io(target, err))?;
    }
    Ok(())
}

fn removed_marker_store(
    path: &Path,
) -> Result<(super::sessionv4::fs::ConfinedRoot, String), ImportError> {
    let marker = session_v4_removed_marker(path);
    if marker.as_os_str().is_empty() {
        return Err(ImportError::Removed);
    }
    let Some(parent) = marker.parent() else {
        return Err(ImportError::Removed);
    };
    let Some(name) = marker.file_name().and_then(|name| name.to_str()) else {
        return Err(ImportError::Removed);
    };
    let store = super::sessionv4::fs::ConfinedRoot::new(parent).map_err(|err| match err {
        V4Error::Io { source, .. } => ImportError::io(parent, source),
        err => ImportError::Source(err),
    })?;
    Ok((store, name.to_string()))
}

fn classify_io(kind: io::ErrorKind) -> SkipReason {
    if kind == io::ErrorKind::PermissionDenied {
        SkipReason::Permission
    } else {
        SkipReason::Corrupt
    }
}

fn classify_read(err: &V4Error) -> SkipReason {
    if error_chain(err).any(|err| {
        err.downcast_ref::<io::Error>()
            .is_some_and(|err| err.kind() == io::ErrorKind::PermissionDenied)
    }) {
        return SkipReason::Permission;
    }
    if has_v4(err, &|err| matches!(err, V4Error::TooLarge(_))) {
        SkipReason::TooLarge
    } else if has_v4(err, &|err| matches!(err, V4Error::Unsupported(_))) {
        SkipReason::SchemaUnsupported
    } else {
        SkipReason::Corrupt
    }
}

fn classify_write(err: &ImportError) -> SkipReason {
    if error_chain(err).any(|err| {
        err.downcast_ref::<io::Error>()
            .is_some_and(|err| err.kind() == io::ErrorKind::PermissionDenied)
    }) {
        SkipReason::Permission
    } else {
        SkipReason::CopyFailed
    }
}

fn has_v4(err: &V4Error, predicate: &dyn Fn(&V4Error) -> bool) -> bool {
    predicate(err)
        || matches!(err, V4Error::Damaged(super::sessionv4::Damage::EventPayload(inner)) if has_v4(inner, predicate))
}

fn is_not_found(err: &V4Error) -> bool {
    error_chain(err).any(|err| {
        err.downcast_ref::<io::Error>()
            .is_some_and(|err| err.kind() == io::ErrorKind::NotFound)
    })
}

fn error_chain<'a>(
    err: &'a (dyn std::error::Error + 'static),
) -> impl Iterator<Item = &'a (dyn std::error::Error + 'static)> + 'a {
    std::iter::successors(Some(err), |current| (*current).source())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[path = "sessionv4/v4_import_tests.rs"]
mod tests;
