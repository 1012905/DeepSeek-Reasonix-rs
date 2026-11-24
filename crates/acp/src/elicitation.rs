//! `elicitation/create` outbound request shapes and the client reply
//! (elicitation.go:79-115).
//!
//! The server assembles one of two parameter payloads depending on the ask
//! mode; the client replies with an action and, for form prompts, per-field
//! content values. This crate freezes the wire shapes only; mode gating and
//! question rendering belong to the runtime.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Reply action: the client accepted the elicited prompt.
pub const ACTION_ACCEPT: &str = "accept";
/// Reply action: the client cancelled the elicited prompt.
pub const ACTION_CANCEL: &str = "cancel";

/// URL-mode prompt: ask the client to open or confirm an external URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ElicitationUrlParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub mode: String,
    #[serde(rename = "elicitationId")]
    pub elicitation_id: String,
    pub url: String,
    pub message: String,
}

/// Form-mode prompt: ask the client to answer a schema-defined set of fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ElicitationFormParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub mode: String,
    pub message: String,
    #[serde(rename = "requestedSchema")]
    pub requested_schema: Value,
}

/// The client's reply to one `elicitation/create` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ElicitationReply {
    pub action: String,
    /// Per-field answers keyed by question id; empty for URL mode.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub content: BTreeMap<String, Value>,
}

impl ElicitationReply {
    /// Whether the reply accepted the prompt; anything else (unknown action,
    /// malformed envelope) counts as a decline, mirroring the oracle's
    /// non-accept branches.
    pub fn accepted(&self) -> bool {
        self.action == ACTION_ACCEPT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_params_round_trip() {
        let params = ElicitationUrlParams {
            session_id: "s1".to_owned(),
            mode: "url".to_owned(),
            elicitation_id: "elicit-q1".to_owned(),
            url: "https://example.com".to_owned(),
            message: "MCP server mcp asks: open link?".to_owned(),
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(value["mode"], "url");
        assert_eq!(value["elicitationId"], "elicit-q1");
        let back: ElicitationUrlParams = serde_json::from_value(value).unwrap();
        assert_eq!(back, params);
    }

    #[test]
    fn form_params_round_trip() {
        let params = ElicitationFormParams {
            session_id: "s1".to_owned(),
            mode: "form".to_owned(),
            message: "MCP server mcp asks: fill the form".to_owned(),
            requested_schema: serde_json::json!({"type": "object"}),
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(
            value["requestedSchema"],
            serde_json::json!({"type": "object"})
        );
        let back: ElicitationFormParams = serde_json::from_value(value).unwrap();
        assert_eq!(back, params);
    }

    #[test]
    fn reply_action_classification() {
        let accept = ElicitationReply {
            action: ACTION_ACCEPT.to_owned(),
            content: BTreeMap::new(),
        };
        assert!(accept.accepted());
        let cancel = ElicitationReply {
            action: ACTION_CANCEL.to_owned(),
            content: BTreeMap::new(),
        };
        assert!(!cancel.accepted());
        let unknown = ElicitationReply {
            action: "deny".to_owned(),
            content: BTreeMap::new(),
        };
        assert!(!unknown.accepted());
    }

    #[test]
    fn reply_content_round_trip() {
        let reply = ElicitationReply {
            action: ACTION_ACCEPT.to_owned(),
            content: BTreeMap::from([("q1".to_owned(), Value::String("option-a".to_owned()))]),
        };
        let value = serde_json::to_value(&reply).unwrap();
        assert_eq!(value["content"]["q1"], "option-a");
        let back: ElicitationReply = serde_json::from_value(value).unwrap();
        assert_eq!(back, reply);
    }
}
