//! The structured ask frame, mirroring `eventwire/ask.go` and the `Ask*`
//! types in `eventwire/wire.go`.

use crate::eventwire::util;
use serde::{Deserialize, Serialize};

/// One structured question the run is paused on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ask {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub questions: Vec<AskQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<AskOrigin>,
    #[serde(rename = "skipInMs", default, skip_serializing_if = "util::is_zero")]
    pub skip_in_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskQuestion {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub header: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default)]
    pub options: Vec<AskOption>,
    #[serde(default, skip_serializing_if = "util::is_false")]
    pub multi: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<crate::surfacespec::Spec>,
    #[serde(rename = "callId", default, skip_serializing_if = "String::is_empty")]
    pub call_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskOption {
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// Where the ask came from. `urlHost` and `urlLocal` are the host's own
/// reading of `url` and ride along so clients need not re-resolve it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskOrigin {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(rename = "urlHost", default, skip_serializing_if = "String::is_empty")]
    pub url_host: String,
    #[serde(rename = "urlLocal", default, skip_serializing_if = "util::is_false")]
    pub url_local: bool,
}
