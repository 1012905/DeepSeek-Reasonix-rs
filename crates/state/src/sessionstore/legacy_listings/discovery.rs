use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;

use super::{
    CancellationToken, LegacyListingError, LegacyReplay, Listing, read_legacy_transcript,
    resolver_for,
};
use crate::sessionstore::eventlog::record::CreatedAt;
use crate::sessionstore::listing::SessionInfo;
use crate::sessionstore::session::session_artifact_exists;
use crate::sessionstore::sessionv4::{Session as V4Session, fs::ConfinedRoot, open_in};
use crate::sessionstore::v4_import::{is_v4_session_removed, v4_copy_begun, v4_import_path};
use crate::store::naming::{is_v4_session_id, session_cleanup_pending, v4_sessions_root};

#[derive(Clone, Debug, Default)]
pub struct LegacyDiscovery {
    pub sources: Vec<V4Session>,
    pub failures: usize,
    pub(crate) seen: BTreeSet<String>,
}

pub fn discover_legacy_sessions(dir: &Path) -> Result<LegacyDiscovery, LegacyListingError> {
    discover_legacy_sessions_where(dir, v4_copy_begun, &CancellationToken::new())
}

pub(crate) fn discover_background_legacy_sessions_with_token(
    dir: &Path,
    token: &CancellationToken,
) -> Result<LegacyDiscovery, LegacyListingError> {
    discover_legacy_sessions_where(dir, session_artifact_exists, token)
}

pub(super) fn discover_legacy_sessions_where(
    dir: &Path,
    imported: impl Fn(&Path) -> bool,
    token: &CancellationToken,
) -> Result<LegacyDiscovery, LegacyListingError> {
    if token.is_cancelled() {
        return Err(LegacyListingError::Cancelled);
    }
    let root = v4_sessions_root(dir);
    if root.as_os_str().is_empty() {
        return Ok(LegacyDiscovery::default());
    }
    let mut entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(LegacyDiscovery::default());
        }
        Err(error) => {
            return Err(LegacyListingError::Io {
                path: root,
                source: error.into(),
            });
        }
    };
    let mut names = Vec::new();
    loop {
        if token.is_cancelled() {
            return Err(LegacyListingError::Cancelled);
        }
        let Some(entry) = entries.next() else {
            break;
        };
        let entry = entry.map_err(|error| LegacyListingError::Io {
            path: root.clone(),
            source: error.into(),
        })?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let kind = entry.file_type().map_err(|error| LegacyListingError::Io {
            path: entry.path(),
            source: error.into(),
        })?;
        if kind.is_dir() && !name.starts_with('.') && is_v4_session_id(&name) {
            names.push(name);
        }
    }
    names.sort();
    let mut discovery = LegacyDiscovery::default();
    for name in names {
        let path = v4_import_path(dir, &name);
        if !legacy_session_visible(&path) || imported(&path) {
            continue;
        }
        if token.is_cancelled() {
            return Err(LegacyListingError::Cancelled);
        }
        discovery.seen.insert(name.clone());
        match open_in(&root, &name) {
            Ok(source) => discovery.sources.push(source),
            Err(_) => discovery.failures += 1,
        }
    }
    Ok(discovery)
}

pub(crate) fn legacy_session_visible(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !marker_present(path, &session_cleanup_pending(path))
        && !is_v4_session_removed(path)
}

fn marker_present(session_path: &Path, marker: &Path) -> bool {
    if marker.as_os_str().is_empty() {
        return false;
    }
    let Some(parent) = session_path.parent() else {
        return false;
    };
    let Some(name) = marker.file_name() else {
        return false;
    };
    ConfinedRoot::new(parent).is_ok_and(|root| root.metadata(Path::new(name)).is_ok())
}

pub fn legacy_session_info(
    dir: &Path,
    source: &V4Session,
    listing: &Listing,
    counts_known: bool,
) -> SessionInfo {
    let updated = source.updated_at();
    SessionInfo {
        path: v4_import_path(dir, &source.manifest.session_id)
            .to_string_lossy()
            .into_owned(),
        created_at: CreatedAt::from_datetime(source.manifest.created_at),
        last_activity_at: CreatedAt::from_datetime(updated),
        mod_time: CreatedAt::from_datetime(updated),
        preview: listing.preview.clone(),
        turns: listing.turns,
        counts_known,
        custom_title: listing.title.clone(),
        ..SessionInfo::default()
    }
}

pub fn sort_legacy_rows(rows: &mut [SessionInfo]) {
    rows.sort_by(|left, right| {
        right
            .last_activity_at
            .get()
            .cmp(&left.last_activity_at.get())
            .then_with(|| left.path.cmp(&right.path))
    });
}

pub fn with_v4_sessions(dir: &Path, mut rows: Vec<SessionInfo>) -> Vec<SessionInfo> {
    if v4_sessions_root(dir).as_os_str().is_empty() {
        return rows;
    }
    let resolver = resolver_for(dir);
    let token = CancellationToken::new();
    let replay: LegacyReplay = Arc::new(read_legacy_transcript);
    let discovery = discover_legacy_sessions(dir).unwrap_or_default();
    for source in discovery.sources {
        let Ok(listing) = resolver.resolve_with_token(&source, Arc::clone(&replay), &token) else {
            continue;
        };
        if listing.turns != 0 {
            rows.push(legacy_session_info(dir, &source, &listing, true));
        }
    }
    let _ = resolver.flush(dir, false);
    sort_legacy_rows(&mut rows);
    rows
}
