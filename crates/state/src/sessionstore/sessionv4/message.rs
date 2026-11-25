use std::collections::HashMap;

use base64::Engine;
use reasonix_contract::provider::{Message, Role, ToolCall};
use serde::Deserialize;
use serde_json::Value;

use super::content::{ContentPool, ContentRef};
use super::error::{Damage, V4Error};

#[derive(Debug, Default, Deserialize)]
struct Extra {
    #[serde(default)]
    id: String,
    #[serde(default)]
    origin: String,
    #[serde(default)]
    image_inputs: Vec<ImageInput>,
}

#[derive(Debug, Default, Deserialize)]
struct ImageInput {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    url: String,
    attachment: Option<Attachment>,
}

#[derive(Debug, Default, Deserialize)]
struct Attachment {
    #[serde(default)]
    content: ContentRef,
}

#[derive(Default, Deserialize)]
struct Core {
    #[serde(default)]
    role: Role,
    #[serde(default)]
    content: String,
    #[serde(rename = "raw_content", default)]
    raw_content: String,
    #[serde(rename = "reasoning_content", default)]
    reasoning_content: String,
    #[serde(rename = "tool_calls", default)]
    tool_calls: Vec<ToolCall>,
    #[serde(rename = "tool_call_id", default)]
    tool_call_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    images: Vec<String>,
    #[serde(rename = "createdAt", default)]
    created_at: i64,
    #[serde(rename = "local_only", default)]
    local_only: bool,
}

#[derive(Clone, Debug)]
pub struct DecodedMessage {
    pub id: String,
    pub message: Message,
}

pub fn decode(raw: &Value, pool: &ContentPool) -> Result<DecodedMessage, V4Error> {
    let extra: Extra = serde_json::from_value(raw.clone()).map_err(Damage::MessageJson)?;
    let decoded = serde_json::from_value::<Message>(raw.clone());
    let mut message = match decoded {
        Ok(message) => message,
        Err(_) => {
            let core: Core = serde_json::from_value(raw.clone()).map_err(Damage::MessageJson)?;
            Message {
                role: core.role,
                content: core.content,
                raw_content: core.raw_content,
                reasoning_content: core.reasoning_content,
                tool_calls: core.tool_calls,
                tool_call_id: core.tool_call_id,
                name: core.name,
                images: core.images,
                created_at: core.created_at,
                local_only: core.local_only,
                ..Message::default()
            }
        }
    };
    message.host_authored |= extra.origin == "host";
    for input in extra.image_inputs {
        match input.kind.as_str() {
            "attachment" => {
                if let Some(attachment) = input.attachment
                    && let Ok(data) = pool.read(&attachment.content)
                {
                    message.images.push(format!(
                        "data:{};base64,{}",
                        attachment.content.media_type,
                        base64::engine::general_purpose::STANDARD.encode(data)
                    ));
                }
            }
            "url" if !input.url.is_empty() => message.images.push(input.url),
            _ => {}
        }
    }
    Ok(DecodedMessage {
        id: extra.id,
        message,
    })
}

#[derive(Default)]
pub struct MessageFold {
    slots: Vec<Option<DecodedMessage>>,
    ids: HashMap<String, usize>,
    empty: Vec<usize>,
}

impl MessageFold {
    fn append(&mut self, message: DecodedMessage) {
        let index = self.slots.len();
        if message.id.is_empty() {
            self.empty.push(index);
        } else {
            self.ids.insert(message.id.clone(), index);
        }
        self.slots.push(Some(message));
    }

    pub fn apply(&mut self, kind: &str, body: &Body, pool: &ContentPool) -> Result<(), V4Error> {
        match kind {
            "message/complete" | "message/upsert" => {
                let message = decode(&body.message, pool)?;
                if !message.id.is_empty()
                    && let Some(index) = self.ids.get(&message.id).copied()
                {
                    if kind == "message/upsert" {
                        self.slots[index] = Some(message);
                    }
                    return Ok(());
                }
                self.append(message);
            }
            "message/retract" => {
                for id in &body.message_ids {
                    if id.is_empty() {
                        for index in self.empty.drain(..) {
                            self.slots[index] = None;
                        }
                    } else if let Some(index) = self.ids.remove(id) {
                        self.slots[index] = None;
                    }
                }
            }
            "history/replace" | "legacy/import" => {
                let mut next = Self::default();
                for raw in &body.messages {
                    let message = decode(raw, pool)?;
                    if message.id.is_empty() || !next.ids.contains_key(&message.id) {
                        next.append(message);
                    }
                }
                *self = next;
            }
            _ => {}
        }
        Ok(())
    }

    pub fn messages(&self) -> Vec<Message> {
        self.slots
            .iter()
            .filter_map(Option::as_ref)
            .map(|entry| entry.message.clone())
            .collect()
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Body {
    #[serde(default)]
    pub messages: Vec<Value>,
    #[serde(default)]
    pub message: Value,
    #[serde(rename = "messageIds", default)]
    pub message_ids: Vec<String>,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "modelRef", default)]
    pub model_ref: String,
}
