//! The guardian assessment frame, mirroring the `Guardian` type in
//! `eventwire/wire.go`. Its JSON names stay snake_case because that is what
/// the Go tags spell and the frontend reads.
use crate::eventwire::usage::Usage;
use crate::eventwire::util;
use serde::{Deserialize, Serialize};

/// The safety guardian's reading of one tool call before it runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guardian {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub outcome: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub risk_level: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user_authorization: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rationale: String,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub duration_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}
