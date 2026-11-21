//! UI wire types referenced by the lifecycle handshake (§6, §12.11).
//!
//! Only the declaration type needed by `InitializeResult` lands in this
//! slice; surface payloads and view trees belong to the structured-UI wave.

use serde::{Deserialize, Serialize};

/// One namespaced UI action an extension declares at initialize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UIActionDecl {
    #[serde(rename = "actionId")]
    pub action_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_decl_round_trips() {
        let decl = UIActionDecl {
            action_id: "acme.refresh".into(),
            label: "Refresh".into(),
        };
        let json = serde_json::to_string(&decl).unwrap();
        assert_eq!(
            json,
            "{\"actionId\":\"acme.refresh\",\"label\":\"Refresh\"}"
        );
        assert_eq!(serde_json::from_str::<UIActionDecl>(&json).unwrap(), decl);
    }
}
