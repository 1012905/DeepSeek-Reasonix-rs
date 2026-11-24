//! `_reasonix.io/session/info` and `_reasonix.io/session/mcpServers` shapes
//! (session_info.go:10-121).
//!
//! The servers list reports name and live flags only: no command, argument, or
//! environment rides the wire. The controller that supplies labels, refs, and
//! cache tallies is runtime work; this module freezes the DTOs and the ratio
//! gate.

use serde::{Deserialize, Serialize};

/// Info / mcpServers schema version.
pub const SESSION_INFO_SCHEMA_VERSION: i32 = 1;

/// Workspace capability scope (capability_scope.go:18-34).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixSessionScope {
    pub root: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub key: String,
    pub repo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trees: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub overrides: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<bool>,
}

/// Whole-session prompt-cache tally (session_info.go:32-36).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixSessionCache {
    #[serde(rename = "hitTokens")]
    pub hit_tokens: i64,
    #[serde(rename = "missTokens")]
    pub miss_tokens: i64,
    #[serde(rename = "hitRatio")]
    pub hit_ratio: Option<f64>,
}

impl ReasonixSessionCache {
    /// Builds the tally; the ratio rides only when hit + miss > 0
    /// (session_info.go:66-79).
    pub fn new(hit: i64, miss: i64) -> Self {
        let hit_ratio = (hit + miss > 0).then(|| hit as f64 / (hit + miss) as f64);
        Self {
            hit_tokens: hit,
            miss_tokens: miss,
            hit_ratio,
        }
    }
}

/// Session identity beside its running state (session_info.go:18-28).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixSessionInfo {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub label: String,
    #[serde(rename = "modelRef")]
    pub model_ref: String,
    #[serde(rename = "workspaceRoot")]
    pub workspace_root: String,
    pub scope: ReasonixSessionScope,
    #[serde(rename = "agentPreset")]
    pub agent_preset: String,
    #[serde(rename = "autoApproveTools")]
    pub auto_approve_tools: bool,
    pub cache: ReasonixSessionCache,
}

/// Addresses one session, and optionally one configured server
/// (session_info.go:39-42).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionMcpServersParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// One configured server's on/off state (session_info.go:52-59).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixConfiguredServer {
    pub name: String,
    pub enabled: bool,
    #[serde(rename = "localOverride")]
    pub local_override: bool,
    pub pending: bool,
    pub changed: bool,
    pub tools: i64,
}

/// Configured-server listing (session_info.go:46-50).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonixMcpServers {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub servers: Vec<ReasonixConfiguredServer>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_ratio_rides_only_when_tally_nonzero() {
        let empty = ReasonixSessionCache::new(0, 0);
        assert!(empty.hit_ratio.is_none());

        let mixed = ReasonixSessionCache::new(3, 1);
        assert_eq!(mixed.hit_ratio, Some(0.75));

        let value = serde_json::to_value(&mixed).unwrap();
        assert_eq!(value["hitTokens"], 3);
        assert_eq!(value["missTokens"], 1);
        assert_eq!(value["hitRatio"], 0.75);
    }

    #[test]
    fn info_tree_keeps_declared_field_order() {
        let info = ReasonixSessionInfo {
            schema_version: SESSION_INFO_SCHEMA_VERSION,
            session_id: "s1".to_owned(),
            label: "main".to_owned(),
            model_ref: "anthropic/claude".to_owned(),
            workspace_root: "/proj".to_owned(),
            scope: ReasonixSessionScope {
                root: "/proj".to_owned(),
                name: "workspace".to_owned(),
                label: None,
                key: "workspace:/proj".to_owned(),
                repo: false,
                trees: None,
                branch: None,
                overrides: 0,
                current: None,
            },
            agent_preset: "builtin:general".to_owned(),
            auto_approve_tools: false,
            cache: ReasonixSessionCache::new(3, 1),
        };
        let text = serde_json::to_string(&info).unwrap();
        let expected = "{\"schemaVersion\":1,\"sessionId\":\"s1\",\"label\":\"main\",\"modelRef\":\"anthropic/claude\",\"workspaceRoot\":\"/proj\",\"scope\":{\"root\":\"/proj\",\"name\":\"workspace\",\"key\":\"workspace:/proj\",\"repo\":false,\"overrides\":0},\"agentPreset\":\"builtin:general\",\"autoApproveTools\":false,\"cache\":{\"hitTokens\":3,\"missTokens\":1,\"hitRatio\":0.75}}";
        assert_eq!(text, expected);
    }

    #[test]
    fn servers_entry_omits_no_command_or_env() {
        let listing = ReasonixMcpServers {
            schema_version: 1,
            session_id: "s1".to_owned(),
            servers: vec![ReasonixConfiguredServer {
                name: "fs".to_owned(),
                enabled: true,
                local_override: false,
                pending: false,
                changed: false,
                tools: 4,
            }],
        };
        let value = serde_json::to_value(&listing).unwrap();
        let entry = &value["servers"][0];
        for forbidden in ["command", "args", "argv", "env", "url", "headers"] {
            assert!(entry.get(forbidden).is_none(), "{} leaked", forbidden);
        }
        assert_eq!(entry["tools"], 4);
    }
}
