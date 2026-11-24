//! `.acp.json` session sidecar schema and the pure write/load rules
//! (service.go:1388-1406; session_meta.go:64-231).
//!
//! The sidecar remembers where a session lives, what it records, and its
//! listing title between connections. Disk IO, atomic replace, and the
//! transcript lease are runtime wiring; this module freezes the twelve-field
//! schema, the path derivation, the load-side backfill, the list beat rule,
//! and the decode error identity. The sidecar deliberately carries no schema
//! version field (matrix B §2.9).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Sidecar filename suffix beside a transcript (`<id>.acp.json`).
pub const SIDECAR_SUFFIX: &str = ".acp.json";
/// Temp file prefix used in the same directory during an atomic write.
pub const TEMP_FILE_PREFIX: &str = ".acp-session.";
/// Indent width for the pretty-printed sidecar body.
pub const WRITE_INDENT: usize = 2;
/// Parent directory mode for a missing sidecar directory.
pub const PARENT_DIR_MODE: u32 = 0o755;

/// The twelve persisted fields (`acpSessionMeta`, service.go:1388-1406).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AcpSessionMeta {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Two-state override: `None` = provider config wins; `Some("")` =
    /// explicit provider default. The two must not be merged.
    #[serde(rename = "effortOverride", skip_serializing_if = "Option::is_none")]
    pub effort_override: Option<String>,
    #[serde(rename = "runtimeProfile", skip_serializing_if = "Option::is_none")]
    pub runtime_profile: Option<String>,
    #[serde(rename = "toolApprovalMode", skip_serializing_if = "Option::is_none")]
    pub tool_approval_mode: Option<String>,
    #[serde(rename = "collaborationMode", skip_serializing_if = "Option::is_none")]
    pub collaboration_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// RFC3339 timestamp; empty string models the zero value.
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// RFC3339 timestamp; empty string models the zero value.
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    /// Persisted status telemetry (`persistedStatusTelemetry`); opaque at this
    /// boundary because its internal counters do not ride the ACP snapshot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Value>,
    /// Redirect marker: recovery branch transcript basename this session lives in.
    #[serde(rename = "activeTranscript", skip_serializing_if = "Option::is_none")]
    pub active_transcript: Option<String>,
}

/// Derives the sidecar path from a transcript path: strip the transcript
/// extension, append `.acp.json` (session_meta.go:193-198).
pub fn meta_path(transcript: &str) -> String {
    if transcript.is_empty() {
        return String::new();
    }
    let ext_len = std::path::Path::new(transcript)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.len() + 1)
        .unwrap_or(0);
    let base = &transcript[..transcript.len() - ext_len];
    format!("{base}{SIDECAR_SUFFIX}")
}

/// Listing title preview: whitespace normalized; at most 80 runes, otherwise
/// the first 77 runes plus `...` (session_meta.go:224-231).
pub fn preview_title(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let runes: Vec<char> = joined.chars().collect();
    if runes.len() <= 80 {
        joined
    } else {
        format!("{}...", runes[..77].iter().collect::<String>())
    }
}

/// Load-side backfill for a loaded session (session_meta.go:64-96). Disk
/// writes are full-structure overwrites; defaults are filled only here.
///
/// * missing id takes the resolved session id;
/// * an explicit cwd overrides the stored cwd;
/// * an empty title takes the history-derived title;
/// * a zero `createdAt` takes `now`; a zero `updatedAt` falls back to
///   `createdAt`.
pub fn backfill_loaded(
    meta: AcpSessionMeta,
    id: &str,
    cwd: &str,
    history_title: &str,
    now: &str,
) -> AcpSessionMeta {
    let session_id = if meta.session_id.is_empty() {
        id.to_owned()
    } else {
        meta.session_id
    };
    let cwd = if !cwd.is_empty() {
        cwd.to_owned()
    } else {
        meta.cwd
    };
    let title = match meta.title {
        Some(t) if !t.is_empty() => Some(t),
        _ => (!history_title.is_empty()).then(|| history_title.to_owned()),
    };
    let created_at = if meta.created_at.is_empty() {
        now.to_owned()
    } else {
        meta.created_at
    };
    let updated_at = if meta.updated_at.is_empty() {
        created_at.clone()
    } else {
        meta.updated_at
    };
    AcpSessionMeta {
        session_id,
        cwd,
        model: meta.model,
        effort_override: meta.effort_override,
        runtime_profile: meta.runtime_profile,
        tool_approval_mode: meta.tool_approval_mode,
        collaboration_mode: meta.collaboration_mode,
        title,
        created_at,
        updated_at,
        status: meta.status,
        active_transcript: meta.active_transcript,
    }
}

/// List beat rule: a live-transcript sidecar always beats a redirect sidecar;
/// between two of the same kind, the later `updatedAt` wins
/// (session_meta.go:204-211). Timestamps compare as RFC3339 strings.
pub fn list_meta_beats(a: &AcpSessionMeta, b: &AcpSessionMeta) -> bool {
    let a_redirect = a
        .active_transcript
        .as_deref()
        .is_some_and(|t| !t.trim().is_empty());
    let b_redirect = b
        .active_transcript
        .as_deref()
        .is_some_and(|t| !t.trim().is_empty());
    if a_redirect != b_redirect {
        return !a_redirect;
    }
    a.updated_at > b.updated_at
}

/// Decode failure identity for the `persistedSessionMeta` path
/// (session_meta.go:112): `decode ACP session metadata <path>: <err>`.
pub fn decode_error(path: &str, err: &str) -> String {
    format!("decode ACP session metadata {path}: {err}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_path_strips_transcript_extension() {
        assert_eq!(
            meta_path("/sessions/abc123.jsonl"),
            "/sessions/abc123.acp.json"
        );
        assert_eq!(meta_path(""), "");
    }

    #[test]
    fn title_preview_caps_at_eighty_runes() {
        assert_eq!(preview_title("  hello   world  "), "hello world");
        let long = "字".repeat(100);
        let preview = preview_title(&long);
        assert!(preview.ends_with("..."));
        assert_eq!(preview.chars().count(), 77 + 3);
    }

    #[test]
    fn backfill_fills_missing_id_cwd_title_and_timestamps() {
        let meta = AcpSessionMeta::default();
        let filled = backfill_loaded(meta, "s1", "/proj", "first prompt", "2026-01-01T00:00:00Z");
        assert_eq!(filled.session_id, "s1");
        assert_eq!(filled.cwd, "/proj");
        assert_eq!(filled.title.as_deref(), Some("first prompt"));
        assert_eq!(filled.created_at, "2026-01-01T00:00:00Z");
        assert_eq!(filled.updated_at, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn live_sidecar_beats_redirect_and_recency_breaks_tie() {
        let live = AcpSessionMeta {
            active_transcript: None,
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
            ..Default::default()
        };
        let redirect = AcpSessionMeta {
            active_transcript: Some("branch.jsonl".to_owned()),
            updated_at: "2026-02-01T00:00:00Z".to_owned(),
            ..Default::default()
        };
        assert!(list_meta_beats(&live, &redirect));
        assert!(!list_meta_beats(&redirect, &live));

        let older = AcpSessionMeta {
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
            ..Default::default()
        };
        let newer = AcpSessionMeta {
            updated_at: "2026-01-02T00:00:00Z".to_owned(),
            ..Default::default()
        };
        assert!(list_meta_beats(&newer, &older));
        assert!(!list_meta_beats(&older, &newer));
    }

    #[test]
    fn decode_error_is_verbatim() {
        assert_eq!(
            decode_error("/sessions/s1.acp.json", "invalid character"),
            "decode ACP session metadata /sessions/s1.acp.json: invalid character"
        );
    }

    #[test]
    fn schema_has_no_version_field() {
        let value = serde_json::to_value(AcpSessionMeta::default()).unwrap();
        assert!(value.get("schemaVersion").is_none());
        assert!(value.get("sessionId").is_some());
        assert!(value.get("activeTranscript").is_none());
    }
}
