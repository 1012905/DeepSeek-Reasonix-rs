//! Provider message contract types (Go:
//! `internal/contract/provider`). Wire shapes are frozen in
//! `docs/contracts/provider-message.md`; client, streaming, retry,
//! usage and vendor dialects belong to the M3 provider-client contract.

mod local_metadata;
mod message;
mod normalize;
mod projection;
mod stream_failure;
mod tool_call;
mod video;

pub use local_metadata::{
    DecisionReceipt, InterruptedToolSummary, InterruptedTurnRecovery, MemoryCitation,
    ToolExecution, ToolFailure, UnavailableImage, Via,
};
pub use message::{INTERRUPTED_TOOL_RESULT, Message};
pub use normalize::{
    args_need_repair, attach_standalone_decision_receipts, backfill_tool_call_names,
    close_truncated_json, id_distinct, needs_tool_call_arg_repair, normalize_messages,
    normalize_session_messages, pair_tool_results, repair_tool_call_args, sanitize_tool_pairing,
    session_tool_results, tool_turn_well_formed,
};
pub use projection::{
    UNAVAILABLE_IMAGES_NOTE, model_messages, projection_messages, unavailable_images_note,
};
pub use stream_failure::{
    STREAM_FAILURE_INVALID_FUNCTION_CALL, STREAM_FAILURE_UNFINISHED_FUNCTION_CALL,
    StreamFailureCause, StreamFailureError,
};
pub use tool_call::{ARGUMENT_ENVELOPE_UNWRAPPED, ArgumentNormalization, ToolCall};
pub use video::{
    VIDEO_FAILED_NOTE, VIDEO_NOT_SENT_NOTE, VIDEO_SHOWN_NOTE, VideoPart, VideoRef, safe_video_name,
    video_failed_note, video_not_sent_note, video_shown_note,
};

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Name of the synthetic local-only tool whose result records durable
/// content that must never reach a provider (Go: `LocalOnlyToolName`).
pub const LOCAL_ONLY_TOOL_NAME: &str = "__reasonix_local_only__";

/// Same value as [LOCAL_ONLY_TOOL_NAME] (Go: `LocalOnlyToolID`).
pub const LOCAL_ONLY_TOOL_ID: &str = "__reasonix_local_only__";

/// A message author role. Any string decodes; the four known roles are
/// behavioral, not wire-validated (Go: `Role`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Role(Cow<'static, str>);

impl Role {
    /// `system` (Go: `RoleSystem`).
    pub const SYSTEM: Role = Role(Cow::Borrowed("system"));
    /// `user` (Go: `RoleUser`).
    pub const USER: Role = Role(Cow::Borrowed("user"));
    /// `assistant` (Go: `RoleAssistant`).
    pub const ASSISTANT: Role = Role(Cow::Borrowed("assistant"));
    /// `tool` (Go: `RoleTool`).
    pub const TOOL: Role = Role(Cow::Borrowed("tool"));

    /// Builds a role from any string.
    pub fn new(role: &str) -> Role {
        Role(Cow::Owned(role.to_owned()))
    }

    /// Role text as stored on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the role is the empty zero value (Go: `Role == ""`).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl PartialEq<str> for Role {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Role {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl From<&'static str> for Role {
    fn from(role: &'static str) -> Role {
        Role(Cow::Borrowed(role))
    }
}

impl From<String> for Role {
    fn from(role: String) -> Role {
        Role(Cow::Owned(role))
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
