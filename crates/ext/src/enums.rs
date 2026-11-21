//! Frozen string enums from `docs/contracts/ext.md` §12.2.
//!
//! Wire values are byte-exact. Serde rejects unknown values on decode so a
//! typo in an extension surfaces at the boundary.

use serde::{Deserialize, Serialize};

/// Immutable identity exchanged in the initialize handshake.
pub const PROTOCOL_ID: &str = "reasonix.extension.v2";
/// Frozen major version of this protocol build.
pub const PROTOCOL_MAJOR: i32 = 2;
/// Wire string form of `PROTOCOL_MAJOR`.
pub const PROTOCOL_VERSION: &str = "2";

/// Message direction, named from the host's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Direction {
    #[serde(rename = "host_to_extension_request")]
    HostToExtensionRequest,
    #[serde(rename = "extension_to_host_request")]
    ExtensionToHostRequest,
    #[serde(rename = "host_to_extension_notification")]
    HostToExtensionNotification,
    #[serde(rename = "extension_to_host_notification")]
    ExtensionToHostNotification,
}

impl Direction {
    /// Reports whether this direction carries no response.
    pub fn is_notification(self) -> bool {
        matches!(
            self,
            Direction::HostToExtensionNotification | Direction::ExtensionToHostNotification
        )
    }
}

/// Operation class of a registered method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationClass {
    #[serde(rename = "lifecycle")]
    Lifecycle,
    #[serde(rename = "intercept")]
    Intercept,
    #[serde(rename = "observation")]
    Observation,
    #[serde(rename = "provider")]
    Provider,
    #[serde(rename = "ui")]
    Ui,
    #[serde(rename = "content")]
    Content,
    #[serde(rename = "tool")]
    Tool,
    #[serde(rename = "session")]
    Session,
}

macro_rules! wire_enum {
    ($name:ident { $($variant:ident => $wire:expr),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $( #[serde(rename = $wire)] $variant ),+
        }

    };
}

wire_enum!(InterceptEvent {
    SessionStart => "session.start",
    SessionEnd => "session.end",
    SessionLoad => "session.load",
    SessionSave => "session.save",
    SessionRotate => "session.rotate",
    InputReceive => "input.receive",
    AgentBeforeStart => "agent.before_start",
    SystemPromptBuild => "system_prompt.build",
    ContextPrepare => "context.prepare",
    ProviderRequest => "provider.request",
    ProviderResponse => "provider.response",
    ToolBefore => "tool.before",
    ToolAfter => "tool.after",
    PermissionDecision => "permission.decision",
    CompactionPrepare => "compaction.prepare",
    CompactionComplete => "compaction.complete",
    FrontendEvent => "frontend.event",
});

impl InterceptEvent {
    /// The 17 frozen hook points, sorted, matching `protocol.InterceptEvents()`.
    pub const ALL: &'static [(&'static str, Self)] = &[
        ("agent.before_start", Self::AgentBeforeStart),
        ("compaction.complete", Self::CompactionComplete),
        ("compaction.prepare", Self::CompactionPrepare),
        ("context.prepare", Self::ContextPrepare),
        ("frontend.event", Self::FrontendEvent),
        ("input.receive", Self::InputReceive),
        ("permission.decision", Self::PermissionDecision),
        ("provider.request", Self::ProviderRequest),
        ("provider.response", Self::ProviderResponse),
        ("session.end", Self::SessionEnd),
        ("session.load", Self::SessionLoad),
        ("session.rotate", Self::SessionRotate),
        ("session.save", Self::SessionSave),
        ("session.start", Self::SessionStart),
        ("system_prompt.build", Self::SystemPromptBuild),
        ("tool.after", Self::ToolAfter),
        ("tool.before", Self::ToolBefore),
    ];
}

wire_enum!(InterceptDecision {
    Continue => "continue",
    Block => "block",
    Replace => "replace",
    Allow => "allow",
    Deny => "deny",
    Ask => "ask",
});

wire_enum!(UIHostKind {
    Tui => "tui",
    Desktop => "desktop",
    Acp => "acp",
    Headless => "headless",
});

wire_enum!(UISurfaceKind {
    Status => "status",
    Card => "card",
    Form => "form",
    Notification => "notification",
    Panel => "panel",
    View => "view",
});

wire_enum!(UIRequestKind {
    Confirm => "confirm",
    Input => "input",
    Select => "select",
    Multiselect => "multiselect",
});

wire_enum!(UISeverity {
    Info => "info",
    Warn => "warn",
    Error => "error",
});

wire_enum!(ProviderRole {
    System => "system",
    User => "user",
    Assistant => "assistant",
    Tool => "tool",
});

wire_enum!(ProviderChunkType {
    Text => "text",
    Reasoning => "reasoning",
    ToolCallStart => "tool_call_start",
    ToolCallArgsDelta => "tool_call_args_delta",
    ToolCall => "tool_call",
    Usage => "usage",
    Done => "done",
    Error => "error",
});

#[allow(clippy::derivable_impls)]
impl Default for ProviderRole {
    fn default() -> Self {
        Self::System
    }
}

#[allow(clippy::derivable_impls)]
impl Default for ProviderChunkType {
    fn default() -> Self {
        Self::Text
    }
}

wire_enum!(ProviderErrorCode {
    ProviderFailed => "provider_failed",
    ProviderInterrupted => "provider_interrupted",
});

wire_enum!(ContentEncoding {
    Utf8 => "utf8",
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intercept_event_count_and_order_match_go() {
        assert_eq!(InterceptEvent::ALL.len(), 17);
        for pair in InterceptEvent::ALL.windows(2) {
            assert!(pair[0].0 < pair[1].0);
        }
        let encoded = serde_json::to_string(&InterceptEvent::AgentBeforeStart).unwrap();
        assert_eq!(encoded, "\"agent.before_start\"");
    }

    #[test]
    fn enums_reject_unknown_wire_values() {
        assert!(serde_json::from_str::<InterceptEvent>("\"bogus\"").is_err());
        assert!(serde_json::from_str::<ProviderChunkType>("\"chunk\"").is_err());
        assert!(serde_json::from_str::<UISeverity>("\"critical\"").is_err());
    }
}
