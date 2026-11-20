//! Request assembly from contract messages to the chat-completions wire (Go:
//! `openai/openai.go` `buildRequest`, `toolCallReasoning`, and the vendor
//! thinking adapters).

use reasonix_contract::provider::{Message, Role};

use crate::effort;
use crate::vendor;
use crate::wire::{
    ChatContent, ChatContentPart, ChatFunction, ChatImageUrl, ChatMessage, ChatRequest, ChatTool,
    ChatToolCall, ChatToolCallExtraContent, GoogleThoughtSignature, ThinkingMode,
};

/// One completion request (Go: `provider.Request`).
#[derive(Debug, Clone, Default)]
pub struct ChatCompletionRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    /// None omits the field; Some sends the value, including 0.
    pub temperature: Option<f64>,
    pub max_tokens: i64,
    /// Per-call reasoning-depth override; applied only when the endpoint's
    /// effort vocabulary accepts it.
    pub effort_override: String,
}

/// A tool definition (Go: `ToolSchema`); `parameters` is raw JSON Schema.
#[derive(Debug, Clone, Default)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: String,
}

/// The strict empty-object schema a no-parameter tool sends (Go:
/// `CanonicalizeSchema(nil)`); key order matches the Go constant.
pub const EMPTY_OBJECT_SCHEMA: &str = r#"{"properties":{},"required":[],"type":"object"}"#;

/// The resolved vendor facts request assembly reads (Go: the `client` wire
/// fields).
#[derive(Debug, Clone, Default)]
pub(crate) struct Dialect {
    pub name: String,
    pub model: String,
    pub base_url: String,
    pub deepseek: bool,
    pub minimax: bool,
    pub zhipu: bool,
    pub zhipu_depth: String,
    pub longcat: bool,
    pub kimi_k3: bool,
    pub thinking_type: String,
    pub effort: String,
    pub request_efforts: Vec<String>,
    pub vision: bool,
    pub vision_detail: String,
    pub max_output_tokens: i64,
    pub auto_max_output: bool,
    pub extra_body: serde_json::Map<String, serde_json::Value>,
    pub open_code_go: bool,
}

/// Builds the wire request (Go: `buildRequest`).
pub(crate) fn build_chat_request(d: &Dialect, req: &ChatCompletionRequest) -> ChatRequest {
    let src = reasonix_contract::provider::sanitize_tool_pairing(&req.messages);
    let mut msgs: Vec<ChatMessage> = Vec::with_capacity(src.len());
    let open_code_go = d.open_code_go;
    // Images returned by tool calls cannot ride the tool message itself —
    // the API accepts only text content parts under role "tool" — so they are
    // carried by a synthetic user message after the tool-result run (Go:
    // `toolMedia`). Video parts are a later slice.
    let mut pending_images: Vec<String> = Vec::new();
    for m in src.iter() {
        if m.role != Role::TOOL {
            flush_tool_images(d, &mut msgs, &mut pending_images);
        }
        let mut cm = ChatMessage {
            role: m.role.as_str().to_owned(),
            tool_call_id: m.tool_call_id.clone(),
            ..ChatMessage::default()
        };
        if m.role == Role::TOOL && !open_code_go {
            // Strict backends (MiMo) 400 a tool result without the key, even
            // empty (#4711); OpenCode Go 400s one that carries it.
            cm.name = Some(m.name.clone());
        }
        cm.reasoning_content = tool_call_reasoning(d, m);
        for tc in &m.tool_calls {
            let mut wire = ChatToolCall {
                id: tc.id.clone(),
                kind: "function".to_owned(),
                function: crate::wire::ChatFunctionPayload {
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                    thought_signature: String::new(),
                },
                ..ChatToolCall::default()
            };
            if !tc.thought_signature.is_empty()
                && vendor::request_carries_thought_signature(&d.base_url, &d.model)
            {
                wire.extra_content = Some(ChatToolCallExtraContent {
                    google: GoogleThoughtSignature {
                        thought_signature: tc.thought_signature.clone(),
                    },
                });
            }
            cm.tool_calls.push(wire);
        }
        cm.content = message_content(d, m);
        msgs.push(cm);
        if m.role == Role::TOOL && d.vision {
            pending_images.extend(m.images.iter().cloned());
        }
    }
    flush_tool_images(d, &mut msgs, &mut pending_images);

    let mut tools: Vec<ChatTool> = Vec::with_capacity(req.tools.len());
    for t in &req.tools {
        let parameters = if t.parameters.trim().is_empty() {
            ChatFunction::raw_parameters(EMPTY_OBJECT_SCHEMA)
        } else {
            ChatFunction::raw_parameters(&t.parameters)
        };
        tools.push(ChatTool {
            kind: "function".to_owned(),
            function: ChatFunction {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters,
            },
        });
    }

    let effort_now = effort::request_effort(&d.effort, &d.request_efforts, &req.effort_override);
    let mut max_output_tokens = req.max_tokens;
    if max_output_tokens == 0 {
        if d.auto_max_output && d.deepseek {
            // Re-resolve so a per-request override can raise 32K to 64K.
            let reasoning_on = d.thinking_type != "disabled"
                && effort_now != "disabled"
                && effort_now != "off"
                && effort_now != "none";
            max_output_tokens = effort::auto_output_budget(reasoning_on, &effort_now);
        } else {
            max_output_tokens = d.max_output_tokens;
        }
    }
    if max_output_tokens < 0 {
        max_output_tokens = 0;
    }

    let mut out = ChatRequest {
        model: d.model.clone(),
        messages: msgs,
        tools,
        tool_choice: String::new(),
        stream: true,
        stream_options: Some(crate::wire::StreamOptions {
            include_usage: true,
        }),
        temperature: req.temperature,
        max_tokens: max_output_tokens,
        max_completion_tokens: 0,
        reasoning_effort: effort::kimi_k3_reasoning_effort(d.kimi_k3, &effort_now),
        thinking: None,
        extra_body: d.extra_body.clone(),
    };
    if d.kimi_k3 {
        // K3 fixes its sampling values and names the output budget
        // max_completion_tokens.
        out.temperature = None;
        out.max_tokens = 0;
        out.max_completion_tokens = max_output_tokens;
        omit_extra_body(
            &mut out.extra_body,
            &[
                "temperature",
                "top_p",
                "n",
                "presence_penalty",
                "frequency_penalty",
                "max_completion_tokens",
            ],
        );
    } else if vendor::is_openai(&d.base_url) {
        // The current OpenAI contract replaces max_tokens with
        // max_completion_tokens; compatible gateways retain max_tokens.
        out.max_tokens = 0;
        out.max_completion_tokens = max_output_tokens;
    } else if d.deepseek {
        // DeepSeek's CoT is controlled by `thinking` plus `reasoning_effort`
        // for depth.
        out.thinking = Some(ThinkingMode {
            kind: disabled_or(d.thinking_type == "disabled", "disabled", "enabled"),
        });
    } else if d.minimax {
        apply_minimax_thinking(d, &mut out, &effort_now);
    } else if d.zhipu {
        apply_zhipu_effort(d, &mut out, &effort_now);
    } else if d.longcat {
        let t = if !d.effort.is_empty() {
            d.effort.clone()
        } else if !d.thinking_type.is_empty() {
            d.thinking_type.clone()
        } else {
            "enabled".to_owned()
        };
        out.thinking = Some(ThinkingMode { kind: t });
        out.reasoning_effort = String::new();
    } else if !d.thinking_type.is_empty() {
        // Generic provider with an explicit `thinking` config field;
        // reasoning_effort, if any, is left untouched.
        out.thinking = Some(ThinkingMode {
            kind: d.thinking_type.clone(),
        });
    }
    out
}

fn disabled_or(flag: bool, if_true: &str, if_false: &str) -> String {
    if flag {
        if_true.to_owned()
    } else {
        if_false.to_owned()
    }
}

/// Appends one user message carrying the images a completed run of tool
/// results returned, then empties the hold (Go: `toolMedia.flush`, image
/// arm).
fn flush_tool_images(d: &Dialect, msgs: &mut Vec<ChatMessage>, pending_images: &mut Vec<String>) {
    if pending_images.is_empty() {
        return;
    }
    let images = std::mem::take(pending_images);
    let mut parts: Vec<ChatContentPart> = Vec::with_capacity(images.len() + 1);
    parts.push(ChatContentPart {
        kind: "text".to_owned(),
        text: "Images returned by the preceding tool call(s):".to_owned(),
        image_url: None,
    });
    for url in images {
        parts.push(ChatContentPart {
            kind: "image_url".to_owned(),
            text: String::new(),
            image_url: Some(ChatImageUrl {
                url,
                detail: d.vision_detail.clone(),
            }),
        });
    }
    msgs.push(ChatMessage {
        role: "user".to_owned(),
        content: Some(ChatContent::Parts(parts)),
        ..ChatMessage::default()
    });
}

/// Message content serialization: vision user turns carry image_url parts,
/// assistant tool_calls-only turns serialize `content: null`, everything else
/// a string (Go: the content switch in `buildRequest` + `imageContentParts`).
fn message_content(d: &Dialect, m: &Message) -> Option<ChatContent> {
    if d.vision && m.role == Role::USER && !m.images.is_empty() {
        let mut parts = Vec::with_capacity(m.images.len() + 1);
        if !m.content.is_empty() {
            parts.push(ChatContentPart {
                kind: "text".to_owned(),
                text: m.content.clone(),
                image_url: None,
            });
        }
        for url in &m.images {
            parts.push(ChatContentPart {
                kind: "image_url".to_owned(),
                text: String::new(),
                image_url: Some(ChatImageUrl {
                    url: url.clone(),
                    detail: d.vision_detail.clone(),
                }),
            });
        }
        return Some(ChatContent::Parts(parts));
    }
    if m.role != Role::ASSISTANT || m.tool_calls.is_empty() || !m.content.is_empty() {
        return Some(ChatContent::Text(m.content.clone()));
    }
    None
}

/// The `reasoning_content` to serialize on an assistant message, None when
/// the endpoint's protocol has no place for it (Go: `toolCallReasoning`).
fn tool_call_reasoning(d: &Dialect, m: &Message) -> Option<String> {
    if m.role != Role::ASSISTANT {
        return None;
    }
    let reasoning = &m.reasoning_content;
    if d.minimax && !reasoning.is_empty() {
        return Some(reasoning.clone());
    }
    if d.kimi_k3 && (!reasoning.is_empty() || !m.tool_calls.is_empty()) {
        // K3 requires the complete assistant message on multi-turn and
        // tool-call requests, including provider-issued reasoning.
        return Some(reasoning.clone());
    }
    if d.deepseek && !m.tool_calls.is_empty() {
        // DeepSeek 400s a tool_calls turn whose reasoning_content key is
        // absent; an empty value passes. Thinking off tolerates any shape.
        if requires_tool_call_reasoning(d) || !reasoning.is_empty() {
            return Some(reasoning.clone());
        }
    }
    if d.zhipu && !reasoning.is_empty() {
        // GLM requires provider-issued reasoning returned unchanged in later
        // history, including after thinking is turned off.
        return Some(reasoning.clone());
    }
    None
}

/// Whether thinking-mode tool_calls turns must carry the reasoning key (Go:
/// `RequiresToolCallReasoning`).
pub(crate) fn requires_tool_call_reasoning(d: &Dialect) -> bool {
    d.deepseek && d.thinking_type != "disabled"
}

/// Whether complete assistant reasoning replays on multi-turn requests (Go:
/// `RequiresReasoningRoundTrip`).
pub(crate) fn requires_reasoning_round_trip(d: &Dialect) -> bool {
    d.minimax || d.kimi_k3 || glm_thinking_enabled(d)
}

/// MiniMax thinking knob (Go: `applyMiniMaxThinking`).
fn apply_minimax_thinking(d: &Dialect, out: &mut ChatRequest, effort_now: &str) {
    if vendor::is_minimax_flash(&d.model) {
        out.thinking = Some(ThinkingMode {
            kind: "adaptive".to_owned(),
        });
        out.reasoning_effort = effort_now.to_owned();
        omit_extra_body(
            &mut out.extra_body,
            &[
                "thinking",
                "reasoning_effort",
                "service_tier",
                "reasoning_split",
            ],
        );
        out.extra_body
            .insert("reasoning_split".to_owned(), serde_json::Value::Bool(true));
        return;
    }
    let mode = if d.effort.is_empty() {
        "adaptive".to_owned()
    } else {
        d.effort.clone()
    };
    out.thinking = Some(ThinkingMode { kind: mode });
    out.reasoning_effort = String::new();
}

/// Whether this GLM request runs thinking on (Go: `glmThinkingEnabled`).
pub(crate) fn glm_thinking_enabled(d: &Dialect) -> bool {
    if !d.zhipu {
        return false;
    }
    let contract = effort::zhipu_effort_contract(&d.zhipu_depth);
    if let Some(c) = &contract {
        if c.forces_thinking() {
            return true;
        }
        if effort::supports_effort(c.thinking_off, &d.effort) {
            return false;
        }
    }
    let t = if d.thinking_type.is_empty() {
        d.effort.as_str()
    } else {
        d.thinking_type.as_str()
    };
    t != "disabled"
}

/// Zhipu thinking knob and depth (Go: `applyZhipuEffort`).
fn apply_zhipu_effort(d: &Dialect, out: &mut ChatRequest, effort_now: &str) {
    let Some(contract) = effort::zhipu_effort_contract(&d.zhipu_depth) else {
        let t = if d.thinking_type.is_empty() {
            if d.effort.is_empty() {
                "enabled".to_owned()
            } else {
                d.effort.clone()
            }
        } else {
            d.thinking_type.clone()
        };
        out.thinking = Some(ThinkingMode { kind: t });
        out.reasoning_effort = String::new();
        return;
    };
    let mut depth = effort_now.to_owned();
    let forces_thinking = contract.forces_thinking();
    if forces_thinking {
        // GLM-5.3 cannot disable thinking, so a persisted off switch becomes
        // the cheapest level the model does accept.
        out.thinking = Some(ThinkingMode {
            kind: "enabled".to_owned(),
        });
        if d.thinking_type == "disabled" {
            depth = contract.disabled_to.to_owned();
        }
    } else if d.thinking_type == "disabled"
        || effort::supports_effort(contract.thinking_off, &depth)
    {
        out.thinking = Some(ThinkingMode {
            kind: "disabled".to_owned(),
        });
        depth = String::new();
    } else {
        out.thinking = Some(ThinkingMode {
            kind: "enabled".to_owned(),
        });
    }
    out.reasoning_effort = depth;
}

/// Drops reserved config extra-body fields a dialect forbids (Go:
/// `omitExtraBodyFields`).
pub(crate) fn omit_extra_body(
    extra: &mut serde_json::Map<String, serde_json::Value>,
    names: &[&str],
) {
    let blocked: Vec<String> = names
        .iter()
        .map(|n| n.trim().to_ascii_lowercase())
        .collect();
    extra.retain(|key, _| !blocked.contains(&key.trim().to_ascii_lowercase()));
}

/// The wire-only assistant tail an automatic DeepSeek continuation appends
/// (Go: `buildPrefixRequest`). Continuation machinery itself is a later
/// slice; the wire shape is kept documented here.
#[allow(dead_code)]
pub(crate) fn build_prefix_request(
    d: &Dialect,
    req: &ChatCompletionRequest,
    content: &str,
    reasoning: &str,
) -> ChatRequest {
    let mut out = build_chat_request(d, req);
    let mut prefix = ChatMessage {
        role: "assistant".to_owned(),
        prefix: true,
        ..ChatMessage::default()
    };
    prefix.content = Some(ChatContent::Text(content.to_owned()));
    if d.deepseek && d.thinking_type != "disabled" {
        prefix.reasoning_content = Some(reasoning.to_owned());
    }
    out.messages.push(prefix);
    out
}
