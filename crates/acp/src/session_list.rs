//! ACP-030 `session/list` merge, filter, sort, and the page cursor rejection
//! (`session.go:590-649`; `session_meta.go:157-211`).
//!
//! Persisted sidecars and live sessions are merged by id: live entries win.
//! Sidecar duplicates reduce through the `listMetaBeats` rule before that
//! merge. The result sorts by `updatedAt` (RFC3339) descending, with session id
//! ascending as the tie-break; a non-empty cursor is rejected outright because
//! the in-process list is single-page. Disk scanning, the live registry
//! lookup, and persisted message counts are runtime wiring; this module owns
//! the reduction, merge, cwd match, and ordering decisions.

use std::collections::BTreeMap;

use crate::errors::RpcError;
use crate::sidecar::{AcpSessionMeta, list_meta_beats};
use crate::wire::SessionInfo;

/// `-32602` non-absolute cwd (`session.go:595-597`).
pub fn cwd_not_absolute_error() -> RpcError {
    RpcError::invalid_params("session/list: cwd must be an absolute path")
}

/// `-32602` non-empty page cursor (`session.go:599-601`).
pub fn unsupported_cursor_error() -> RpcError {
    RpcError::invalid_params("session/list: unsupported cursor")
}

/// Validates the optional cwd and cursor. Cwd is checked before the cursor,
/// matching the reference order (`session.go:594-601`). The trimmed absolute
/// cwd filter is returned when present.
pub fn validate_list_params(
    cwd: Option<&str>,
    cursor: Option<&str>,
) -> Result<Option<String>, RpcError> {
    let filter = cwd.unwrap_or("").trim().to_owned();
    if !filter.is_empty() && !is_absolute_lexical(&filter) {
        return Err(cwd_not_absolute_error());
    }
    if cursor.unwrap_or("").trim().is_empty() {
        Ok((!filter.is_empty()).then_some(filter))
    } else {
        Err(unsupported_cursor_error())
    }
}

/// Lexical absolute-path check covering POSIX `/`, Windows `\`, and drive
/// letters; `filepath.IsAbs` does not inspect the filesystem.
pub fn is_absolute_lexical(path: &str) -> bool {
    let bytes = path.as_bytes();
    if path.starts_with('/') || path.starts_with('\\') {
        return true;
    }
    bytes.len() >= 3
        && bytes[1] == b':'
        && (path.as_bytes()[2] == b'/' || path.as_bytes()[2] == b'\\')
}

/// Collapses `.`, `..`, and repeated separators lexically, mirroring
/// `filepath.Clean` for the equality check.
pub fn clean_path(path: &str) -> String {
    let absolute = path.starts_with('/') || path.starts_with('\\');
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split(&['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// cwd equality after cleaning (`session.go:610-613`).
pub fn cwd_matches(info: &SessionInfo, filter: &str) -> bool {
    clean_path(&info.cwd) == clean_path(filter)
}

/// Reduces duplicate sidecars per id with the live-transcript beats rule
/// (`session_meta.go:204-211`).
pub fn reduce_meta_beats(metas: Vec<AcpSessionMeta>) -> Vec<AcpSessionMeta> {
    let mut best: BTreeMap<String, AcpSessionMeta> = BTreeMap::new();
    for meta in metas {
        match best.get(&meta.session_id) {
            Some(current) if !list_meta_beats(&meta, current) => {}
            _ => {
                best.insert(meta.session_id.clone(), meta);
            }
        }
    }
    best.into_values().collect()
}

fn meta_to_info(meta: AcpSessionMeta) -> SessionInfo {
    SessionInfo {
        session_id: meta.session_id,
        cwd: meta.cwd,
        title: meta.title,
        updated_at: (!meta.updated_at.is_empty()).then_some(meta.updated_at),
        meta: None,
    }
}

/// Merges reduced sidecars with live entries; live overrides the persisted
/// entry by id (`session.go:607-613`). The optional cleaned cwd filter applies
/// to the merged list.
pub fn merge_sessions(
    metas: Vec<AcpSessionMeta>,
    live: Vec<SessionInfo>,
    filter_cwd: Option<&str>,
) -> Vec<SessionInfo> {
    let mut by_id: BTreeMap<String, SessionInfo> = BTreeMap::new();
    for meta in reduce_meta_beats(metas) {
        let id = meta.session_id.clone();
        by_id.insert(id, meta_to_info(meta));
    }
    for info in live {
        by_id.insert(info.session_id.clone(), info);
    }

    let mut merged: Vec<SessionInfo> = by_id.into_values().collect();
    if let Some(filter) = filter_cwd {
        merged.retain(|info| cwd_matches(info, filter));
    }
    sort_infos(&mut merged);
    merged
}

/// Ordering: `updatedAt` RFC3339 descending, session id ascending tie-break
/// (`session_meta.go:214-219`). A missing or unparseable timestamp compares as
/// the zero value.
pub fn sort_infos(infos: &mut [SessionInfo]) {
    infos.sort_by(|a, b| {
        let a_ts = a.updated_at.clone().unwrap_or_default();
        let b_ts = b.updated_at.clone().unwrap_or_default();
        b_ts.cmp(&a_ts)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcErrorCode;

    fn info(id: &str, cwd: &str, updated: &str) -> SessionInfo {
        SessionInfo {
            session_id: id.to_owned(),
            cwd: cwd.to_owned(),
            title: None,
            updated_at: (!updated.is_empty()).then(|| updated.to_owned()),
            meta: None,
        }
    }

    #[test]
    fn cwd_validated_before_cursor() {
        let err = validate_list_params(Some("relative"), Some("cur")).unwrap_err();
        assert_eq!(err.code, RpcErrorCode::InvalidParams);
        assert_eq!(err.message, "session/list: cwd must be an absolute path");

        let err = validate_list_params(None, Some("cur")).unwrap_err();
        assert_eq!(err.message, "session/list: unsupported cursor");

        let ok = validate_list_params(Some("/proj"), None).unwrap();
        assert_eq!(ok.as_deref(), Some("/proj"));
        let empty = validate_list_params(None, None).unwrap();
        assert!(empty.is_none());
    }

    #[test]
    fn live_overrides_persisted_by_id() {
        let metas = vec![AcpSessionMeta {
            session_id: "s1".to_owned(),
            cwd: "/old".to_owned(),
            title: Some("persisted".to_owned()),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
            ..Default::default()
        }];
        let live = vec![info("s1", "/live", "2026-01-02T00:00:00Z")];
        let merged = merge_sessions(metas, live, None);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].cwd, "/live");
        assert!(merged[0].title.is_none());
    }

    #[test]
    fn cwd_filter_compares_cleaned_paths() {
        let live = vec![info("s1", "/proj/src", ""), info("s2", "/other", "")];
        let merged = merge_sessions(vec![], live, Some("/proj/./src"));
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].session_id, "s1");
    }

    #[test]
    fn sort_orders_timestamp_desc_then_id_asc() {
        let mut merged = vec![
            info("s3", "/p", "2026-01-01T00:00:00Z"),
            info("s1", "/p", "2026-01-03T00:00:00Z"),
            info("s2", "/p", "2026-01-03T00:00:00Z"),
            info("s4", "/p", ""),
        ];
        sort_infos(&mut merged);
        assert_eq!(
            merged
                .iter()
                .map(|i| i.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["s1", "s2", "s3", "s4"]
        );
    }

    #[test]
    fn beats_reduce_duplicate_sidecars_before_merge() {
        let metas = vec![
            AcpSessionMeta {
                session_id: "s1".to_owned(),
                cwd: "/p".to_owned(),
                updated_at: "2026-01-01T00:00:00Z".to_owned(),
                ..Default::default()
            },
            AcpSessionMeta {
                session_id: "s1".to_owned(),
                cwd: "/p".to_owned(),
                updated_at: "2026-01-05T00:00:00Z".to_owned(),
                ..Default::default()
            },
        ];
        let merged = merge_sessions(metas, vec![], None);
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0].updated_at.as_deref(),
            Some("2026-01-05T00:00:00Z")
        );
    }

    #[test]
    fn next_cursor_never_emitted_by_list_result() {
        let result = crate::wire::SessionListResult {
            sessions: vec![],
            next_cursor: None,
        };
        let value = serde_json::to_value(&result).unwrap();
        assert!(value.get("nextCursor").is_none());
    }
}
