//! The workspace lease frame and message attachments, mirroring
//! `eventwire/workspace_lease.go` and `eventwire/attachment.go`.

use crate::eventwire::util;
use serde::{Deserialize, Serialize};

/// Who holds the workspace and how long it has been held or contended.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceLease {
    #[serde(default)]
    pub contended: i64,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub reported: i64,
    #[serde(rename = "waitedMs", default, skip_serializing_if = "util::is_zero")]
    pub waited_ms: i64,
    #[serde(rename = "heldMs", default)]
    pub held_ms: i64,
    #[serde(rename = "idleMs", default)]
    pub idle_ms: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub holder: String,
    #[serde(
        rename = "holderSessionId",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub holder_session_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(
        rename = "requestedPaths",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub requested_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub repository: String,
}

/// A stored image a message references; only its path rides the wire.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    #[serde(default)]
    pub path: String,
}
