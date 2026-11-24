//! `fs/read_text_file` and `fs/write_text_file` capability gating
//! (clientio.go:61-88; protocol.go:671-689).
//!
//! The degradation is asymmetric: a read falls back to the local disk on any
//! miss (missing capability, transport error, or bad reply), while a write only
//! falls back when the capability is absent. With the capability present, a
//! client error is a real write failure that must surface — falling back could
//! double-apply the write.

use serde::{Deserialize, Serialize};

use crate::caps::ClientCapabilities;

/// `fs/read_text_file` params (protocol.go:671-676). Reasonix always reads
/// whole files and pages locally, so `line`/`limit` are never sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FsReadTextFileParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
}

/// `fs/read_text_file` result: the file content only, no pagination or
/// truncation fields (protocol.go:679-681).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FsReadTextFileResult {
    pub content: String,
}

/// `fs/write_text_file` params (protocol.go:685-689).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FsWriteTextFileParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub path: String,
    pub content: String,
}

/// Why a read did not come from the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadFailure {
    /// Client did not advertise `readTextFile`.
    CapabilityAbsent,
    /// Transport / client request failed.
    Transport,
    /// Reply body did not parse as a read result.
    BadReply,
}

/// Outcome of a read attempt from the tool's point of view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOutcome {
    /// Use the local disk; the client view was unavailable for the reason given.
    LocalFallback(ReadFailure),
    /// Carry the client's view, including unsaved editor buffers.
    Client(String),
}

/// Resolves a read against the declared capabilities and the client outcome.
/// Every miss degrades to the local fallback; the read path never raises to
/// the tool (clientio.go:61-74).
pub fn resolve_read(caps: &ClientCapabilities, client: Result<&str, ReadFailure>) -> ReadOutcome {
    if !caps.read_text_file() {
        return ReadOutcome::LocalFallback(ReadFailure::CapabilityAbsent);
    }
    match client {
        Ok(content) => ReadOutcome::Client(content.to_owned()),
        Err(failure) => ReadOutcome::LocalFallback(failure),
    }
}

/// Outcome of a write attempt from the tool's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// Capability absent: proceed with the local write.
    LocalFallback,
    /// Capability present and the client accepted the write.
    Written,
    /// Capability present but the client/transport failed: surface the error,
    /// do not write locally (double-apply guard).
    SurfacedError,
}

/// Resolves a write against the declared capabilities. A missing capability
/// falls back locally; a present capability that errors surfaces instead
/// (clientio.go:80-88).
pub fn resolve_write(caps: &ClientCapabilities, client_error: bool) -> WriteOutcome {
    if !caps.write_text_file() {
        return WriteOutcome::LocalFallback;
    }
    if client_error {
        WriteOutcome::SurfacedError
    } else {
        WriteOutcome::Written
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::FsCapabilities;

    fn caps(read: bool, write: bool) -> ClientCapabilities {
        ClientCapabilities {
            fs: Some(FsCapabilities {
                read_text_file: read,
                write_text_file: write,
            }),
            terminal: false,
            elicitation: None,
            meta: None,
        }
    }

    #[test]
    fn read_degrades_on_missing_capability_transport_or_parse() {
        let no_cap = caps(false, false);
        assert!(matches!(
            resolve_read(&no_cap, Ok("ignored")),
            ReadOutcome::LocalFallback(ReadFailure::CapabilityAbsent)
        ));

        let with_cap = caps(true, false);
        assert!(matches!(
            resolve_read(&with_cap, Err(ReadFailure::Transport)),
            ReadOutcome::LocalFallback(ReadFailure::Transport)
        ));
        assert!(matches!(
            resolve_read(&with_cap, Err(ReadFailure::BadReply)),
            ReadOutcome::LocalFallback(ReadFailure::BadReply)
        ));
        assert_eq!(
            resolve_read(&with_cap, Ok("client view")),
            ReadOutcome::Client("client view".to_owned())
        );
    }

    #[test]
    fn write_falls_back_only_when_capability_absent() {
        let no_cap = caps(false, false);
        assert_eq!(resolve_write(&no_cap, true), WriteOutcome::LocalFallback);
        assert_eq!(resolve_write(&no_cap, false), WriteOutcome::LocalFallback);

        let with_cap = caps(false, true);
        assert_eq!(resolve_write(&with_cap, false), WriteOutcome::Written);
        assert_eq!(resolve_write(&with_cap, true), WriteOutcome::SurfacedError);
    }

    #[test]
    fn read_params_omit_line_and_limit_when_unset() {
        let params = FsReadTextFileParams {
            session_id: "s1".to_owned(),
            path: "src/main.rs".to_owned(),
            line: None,
            limit: None,
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(value["sessionId"], "s1");
        assert_eq!(value["path"], "src/main.rs");
        assert!(value.get("line").is_none());
        assert!(value.get("limit").is_none());
    }

    #[test]
    fn read_result_carries_only_content() {
        let result = FsReadTextFileResult {
            content: "hello".to_owned(),
        };
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value, serde_json::json!({"content": "hello"}));
    }
}
