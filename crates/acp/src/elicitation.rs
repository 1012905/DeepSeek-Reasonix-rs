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

/// Message / description rune cap (elicitation.go:14, 63, 111).
pub const ELICIT_MESSAGE_LIMIT: usize = 1024;
/// Form value / `maxLength` rune cap (elicitation.go:15, 133, 178).
pub const ELICIT_VALUE_LIMIT: usize = 4096;
/// Schema field title rune cap (elicitation.go:110).
pub const ELICIT_TITLE_LIMIT: usize = 120;

// ---- Question model and schema builder -----------------------------------------

/// One form question the adapter renders into `requestedSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AskQuestion {
    pub id: String,
    pub header: String,
    pub prompt: String,
    pub options: Vec<String>,
    pub multi: bool,
    pub default: Vec<String>,
}

/// Strips control characters (newline kept) and caps the rune count, appending
/// the ellipsis when clipped. The oracle also drops Unicode Cf format
/// characters; the port filters control classes and treats the remainder as
/// display text (elicitation.go:62-77).
pub fn clip_plain(text: &str, limit: usize) -> String {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    let runes: Vec<char> = cleaned.chars().collect();
    if runes.len() <= limit {
        cleaned
    } else {
        format!("{}…", runes[..limit].iter().collect::<String>())
    }
}

/// Builds the form `requestedSchema` (elicitation.go:107-141). The field name
/// is the question id; the field type follows the multi/options rules.
pub fn build_schema(questions: &[AskQuestion]) -> Value {
    let mut properties = serde_json::Map::new();
    for question in questions {
        let title = clip_plain(
            if question.header.trim().is_empty() {
                &question.id
            } else {
                &question.header
            },
            ELICIT_TITLE_LIMIT,
        );
        let mut field = serde_json::Map::new();
        field.insert("title".to_owned(), Value::String(title));
        let description = clip_plain(&question.prompt, ELICIT_MESSAGE_LIMIT);
        if !description.is_empty() {
            field.insert("description".to_owned(), Value::String(description));
        }

        let has_options = !question.options.is_empty();
        if question.multi && has_options {
            let mut items = serde_json::Map::new();
            items.insert("type".to_owned(), Value::String("string".to_owned()));
            items.insert(
                "enum".to_owned(),
                Value::Array(
                    question
                        .options
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
            field.insert("type".to_owned(), Value::String("array".to_owned()));
            field.insert("items".to_owned(), Value::Object(items));
            if let Some(first) = question.default.first() {
                field.insert(
                    "default".to_owned(),
                    Value::Array(vec![Value::String(first.clone())]),
                );
            }
        } else if has_options {
            field.insert("type".to_owned(), Value::String("string".to_owned()));
            field.insert(
                "enum".to_owned(),
                Value::Array(
                    question
                        .options
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
            if let Some(first) = question.default.first() {
                field.insert("default".to_owned(), Value::String(first.clone()));
            }
        } else {
            field.insert("type".to_owned(), Value::String("string".to_owned()));
            field.insert(
                "maxLength".to_owned(),
                Value::from(ELICIT_VALUE_LIMIT as i64),
            );
            if let Some(first) = question.default.first() {
                field.insert("default".to_owned(), Value::String(first.clone()));
            }
        }
        properties.insert(question.id.clone(), Value::Object(field));
    }
    serde_json::json!({ "type": "object", "properties": properties })
}

// ---- Mode gating -----------------------------------------------------------------

/// Which origin an ask came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskOrigin {
    /// No origin; the adapter declines outright.
    None,
    /// External page; routes to URL mode.
    Url,
    /// Inline form; routes to form mode.
    Form,
}

/// Advertised mode presence: a key counts only when present and neither empty
/// nor null (elicitation.go:216-219).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AdvertisedModes {
    pub form: bool,
    pub url: bool,
}

/// Decides whether an ask may be offered. URL mode needs the url mode declared
/// and exactly one question; form mode needs the form mode declared and at
/// least one question. A page interaction never degrades to a form, and an
/// unadvertised url client never receives a page (elicitation.go:27-44).
pub fn mode_eligible(origin: AskOrigin, modes: AdvertisedModes, question_count: usize) -> bool {
    match origin {
        AskOrigin::None => false,
        AskOrigin::Url => modes.url && question_count == 1,
        AskOrigin::Form => modes.form && question_count >= 1,
    }
}

// ---- Reply validation -------------------------------------------------------------

/// One validated form answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ValidatedAnswer {
    #[serde(rename = "questionId")]
    pub question_id: String,
    pub selected: Vec<String>,
}

fn content_values(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::String(s) => Some(vec![s.clone()]),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::String(s) => out.push(s.clone()),
                    _ => return None,
                }
            }
            Some(out)
        }
        _ => None,
    }
}

/// Validates a form reply on the adapter side. Any violation rejects the whole
/// form; unselected content is never forwarded to the server
/// (elicitation.go:99-103, 154-187).
pub fn validate_form_answers(
    questions: &[AskQuestion],
    content: &BTreeMap<String, Value>,
) -> Option<Vec<ValidatedAnswer>> {
    let mut answers = Vec::with_capacity(questions.len());
    for question in questions {
        let raw = content.get(&question.id)?;
        let selected = content_values(raw)?;
        if selected
            .iter()
            .any(|v| v.chars().count() > ELICIT_VALUE_LIMIT)
        {
            return None;
        }
        if !question.options.is_empty() && !selected.iter().all(|v| question.options.contains(v)) {
            return None;
        }
        if !question.multi && selected.len() > 1 {
            return None;
        }
        answers.push(ValidatedAnswer {
            question_id: question.id.clone(),
            selected,
        });
    }
    Some(answers)
}

/// URL-mode reply: `accept` or `cancel` answers one question; any other action
/// (including malformed envelopes) declines. `cancel` is a distinct explicit
/// selection, not a decline (elicitation.go:87-91).
pub fn url_answer(question_id: &str, action: &str) -> Option<ValidatedAnswer> {
    match action {
        ACTION_ACCEPT | ACTION_CANCEL => Some(ValidatedAnswer {
            question_id: question_id.to_owned(),
            selected: vec![action.to_owned()],
        }),
        _ => None,
    }
}

/// Builds the outbound message prefix: `MCP server <source> asks: <message>`,
/// clipped to the message limit (elicitation.go:62-66).
pub fn elicit_message(source: &str, message: &str) -> String {
    clip_plain(
        &format!("MCP server {source} asks: {message}"),
        ELICIT_MESSAGE_LIMIT,
    )
}

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
