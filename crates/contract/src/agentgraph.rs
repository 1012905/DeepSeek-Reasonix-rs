//! Mirrors `internal/contract/agentgraph` — the execution graph delta on the wire.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    #[serde(rename = "parentId", default, skip_serializing_if = "String::is_empty")]
    pub parent_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub state: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effort: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub grant: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub wait: String,
    #[serde(rename = "ref", default, skip_serializing_if = "String::is_empty")]
    pub r#ref: String,
    #[serde(
        rename = "queuedAt",
        default,
        skip_serializing_if = "crate::eventwire::util::is_zero"
    )]
    pub queued_at: i64,
    #[serde(
        rename = "startedAt",
        default,
        skip_serializing_if = "crate::eventwire::util::is_zero"
    )]
    pub started_at: i64,
    #[serde(
        rename = "endedAt",
        default,
        skip_serializing_if = "crate::eventwire::util::is_zero"
    )]
    pub ended_at: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub err: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delta {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<Node>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<Node>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<Edge>,
}
