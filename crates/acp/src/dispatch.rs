//! Injectable handler registry and the pure event → `session/update` mapping
//! table (server.go:212-237; dispatch.go:138-251, 692-718).
//!
//! The dispatcher holds boxed handlers keyed by method name; it does not open
//! any transport. The mapping table decides which ACP message kind an internal
//! event becomes. Real event-source wiring, controller calls, and the sink that
//! writes frames are session-runtime concerns marked WIP at this boundary.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::errors::RpcError;
use crate::update::ToolCallLocation;

// ---- Handler traits ----------------------------------------------------------

/// A request handler: turns params into a result value or a typed RPC error.
pub trait RequestHandler {
    fn call(&self, params: Option<Value>) -> Result<Value, RpcError>;
}

/// A notification handler: fire and forget, no reply frame is expected.
pub trait NotificationHandler {
    fn call(&self, params: Option<Value>);
}

/// Where a registered method routes.
pub enum Route<'a> {
    Request(&'a dyn RequestHandler),
    Notification(&'a dyn NotificationHandler),
    NotFound,
}

/// Method table wired once at startup.
#[derive(Default)]
pub struct Dispatcher {
    requests: HashMap<String, Box<dyn RequestHandler>>,
    notifications: HashMap<String, Box<dyn NotificationHandler>>,
}

impl Dispatcher {
    pub fn register_request(&mut self, method: &str, handler: Box<dyn RequestHandler>) {
        self.requests.insert(method.to_owned(), handler);
    }

    pub fn register_notification(&mut self, method: &str, handler: Box<dyn NotificationHandler>) {
        self.notifications.insert(method.to_owned(), handler);
    }

    /// Resolves a method to its registered route. Unregistered methods fall to
    /// [`Route::NotFound`], which the caller answers `-32601`.
    pub fn route(&self, method: &str) -> Route<'_> {
        if let Some(handler) = self.requests.get(method) {
            return Route::Request(handler.as_ref());
        }
        if let Some(handler) = self.notifications.get(method) {
            return Route::Notification(handler.as_ref());
        }
        Route::NotFound
    }

    pub fn request_count(&self) -> usize {
        self.requests.len()
    }

    pub fn notification_count(&self) -> usize {
        self.notifications.len()
    }
}

// ---- Event → update mapping ---------------------------------------------------

/// Internal event kinds recognized by the update sink. Names mirror the event
/// package; dropped kinds are listed here so the table can assert they emit
/// nothing (matrix B §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentEventKind {
    McpSurfaceReady,
    Reasoning,
    Text,
    StreamAttempt,
    ToolDispatch,
    ToolResult,
    Notice,
    CompactionDone,
    ApprovalRequest,
    AskRequest,
    ExtensionSurface,
    ExtensionStatus,
    // Dropped: no ACP message.
    TurnStarted,
    Message,
    Usage,
    Phase,
    TurnDone,
    CompactionStarted,
    ToolProgress,
    Retrying,
    Steer,
    GuardianAssessment,
    ContextMaintenance,
    TodoProgress,
    WorkspaceLease,
}

/// The ACP destination an event maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventDispatch {
    /// MCP surface snapshot notification, gated by the statusUpdates opt-in.
    PublishMcpStatus,
    /// `agent_thought_chunk`.
    ThoughtChunk,
    /// `agent_message_chunk`.
    MessageChunk,
    /// Bookkeeping only; no partial tool card is published.
    BookkeepingOnly,
    /// `tool_call` pending card (partial/refreshed dispatch skipped upstream).
    ToolCallPending,
    /// `tool_call_update` completed/failed.
    ToolCallUpdate,
    /// Warning chunk with the fixed two-newline prefix.
    WarningChunk,
    /// Compaction notice chunk.
    CompactionChunk,
    /// `session/request_permission` round-trip on a worker.
    PermissionRequest,
    /// Ask round-trip: MCP elicitation or per-question permission prompts.
    AskRoundTrip,
    /// Structured extension surface plus flattened text fallback.
    ExtensionDualEmission,
    /// No ACP message for this event kind.
    NoUpdate,
}

/// Maps an event kind to its dispatch destination (dispatch.go:142-251).
pub fn map_event(kind: AgentEventKind) -> EventDispatch {
    match kind {
        AgentEventKind::McpSurfaceReady => EventDispatch::PublishMcpStatus,
        AgentEventKind::Reasoning => EventDispatch::ThoughtChunk,
        AgentEventKind::Text => EventDispatch::MessageChunk,
        AgentEventKind::StreamAttempt => EventDispatch::BookkeepingOnly,
        AgentEventKind::ToolDispatch => EventDispatch::ToolCallPending,
        AgentEventKind::ToolResult => EventDispatch::ToolCallUpdate,
        AgentEventKind::Notice => EventDispatch::WarningChunk,
        AgentEventKind::CompactionDone => EventDispatch::CompactionChunk,
        AgentEventKind::ApprovalRequest => EventDispatch::PermissionRequest,
        AgentEventKind::AskRequest => EventDispatch::AskRoundTrip,
        AgentEventKind::ExtensionSurface | AgentEventKind::ExtensionStatus => {
            EventDispatch::ExtensionDualEmission
        }
        AgentEventKind::TurnStarted
        | AgentEventKind::Message
        | AgentEventKind::Usage
        | AgentEventKind::Phase
        | AgentEventKind::TurnDone
        | AgentEventKind::CompactionStarted
        | AgentEventKind::ToolProgress
        | AgentEventKind::Retrying
        | AgentEventKind::Steer
        | AgentEventKind::GuardianAssessment
        | AgentEventKind::ContextMaintenance
        | AgentEventKind::TodoProgress
        | AgentEventKind::WorkspaceLease => EventDispatch::NoUpdate,
    }
}

// ---- Tool result text shaping --------------------------------------------------

/// Display-only clip budget for a tool result crossing the wire
/// (`maxResultChars`, dispatch.go:34). The full result still reaches the model.
pub const MAX_RESULT_CHARS: usize = 8000;

fn longest_backtick_run(body: &str) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for byte in body.as_bytes() {
        if *byte == b'`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

/// Clips a plain result to [`MAX_RESULT_CHARS`] at a UTF-8 boundary, appending
/// the fixed note with the dropped byte count (dispatch.go:676-686).
pub fn clip(text: &str) -> String {
    let bytes = text.as_bytes();
    if bytes.len() <= MAX_RESULT_CHARS {
        return text.to_owned();
    }
    let mut end = MAX_RESULT_CHARS;
    while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
        end -= 1;
    }
    let dropped = bytes.len() - end;
    format!("{}\n…({dropped} more chars truncated)", &text[..end])
}

/// Shapes a tool result for display. A whole unified diff passes through
/// untruncated inside a ```diff fence whose backtick run outruns the body's
/// longest backtick run; everything else is clipped (dispatch.go:650-673).
pub fn tool_result_text(text: &str, is_diff: bool) -> String {
    if !is_diff {
        return clip(text);
    }
    let body = text.trim_end_matches('\n');
    let fence = "`".repeat(3.max(longest_backtick_run(body) + 1));
    format!("{fence}diff\n{body}\n{fence}")
}

/// Raw args ride as `rawInput` only when they are already valid JSON; an empty
/// or malformed string omits the field (dispatch.go:639-644).
pub fn raw_input(args: &str) -> Option<Value> {
    if args.is_empty() {
        return None;
    }
    serde_json::from_str(args).ok()
}

// ---- Tool call locations -------------------------------------------------------

/// Builtin tools whose `path` argument is a real file target worth a
/// follow-along location; search/list tools are excluded (dispatch.go:723-763).
pub const LOCATION_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "multi_edit",
    "notebook_edit",
    "delete_range",
    "delete_symbol",
    "code_index",
];

fn absolute_path(cwd: &str, path: &str) -> String {
    if path.starts_with('/') || cwd.is_empty() {
        return path.to_owned();
    }
    format!("{}/{}", cwd.trim_end_matches('/'), path)
}

#[derive(Deserialize)]
struct LocationArgs {
    path: String,
    #[serde(default)]
    offset: i32,
}

/// Derives the location pointer from a tool's raw args. Unknown tools and
/// path-less args yield `None`; `read_file`'s 0-based offset surfaces as a
/// 1-based line (dispatch.go:723-763).
pub fn tool_locations(name: &str, raw_args: &str, cwd: &str) -> Option<Vec<ToolCallLocation>> {
    if !LOCATION_TOOLS.contains(&name) {
        return None;
    }
    let args: LocationArgs = serde_json::from_str(raw_args).ok()?;
    if args.path.trim().is_empty() {
        return None;
    }
    let line = (name == "read_file" && args.offset > 0).then_some(args.offset + 1);
    Some(vec![ToolCallLocation {
        path: absolute_path(cwd, &args.path),
        line,
    }])
}

// ---- Tool kind heuristics ------------------------------------------------------

/// Maps a tool name to the coarse kind on the `tool_call` card
/// (dispatch.go:692-718). Explicit wins over the substring heuristic.
pub fn tool_kind(tool_name: &str) -> &'static str {
    match tool_name {
        "read_file" | "ls" | "glob" => return "read",
        "grep" => return "search",
        "edit_file" | "move_file" | "multiedit" | "write_file" => return "edit",
        "bash" | "sandbox_escape" => return "execute",
        _ => {}
    }
    let lower = tool_name.to_ascii_lowercase();
    if lower.contains("search") || lower.contains("grep") || lower.contains("find") {
        "search"
    } else if lower.contains("edit") || lower.contains("write") || lower.contains("replace") {
        "edit"
    } else if lower.contains("read") || lower.contains("cat") || lower.contains("view") {
        "read"
    } else if lower.contains("bash")
        || lower.contains("exec")
        || lower.contains("shell")
        || lower.contains("run")
    {
        "execute"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoRequest;
    impl RequestHandler for EchoRequest {
        fn call(&self, params: Option<Value>) -> Result<Value, RpcError> {
            Ok(params.unwrap_or(Value::Null))
        }
    }

    #[test]
    fn dispatcher_routes_registered_methods_and_not_found() {
        let mut dispatcher = Dispatcher::default();
        dispatcher.register_request("session/new", Box::new(EchoRequest));
        assert!(matches!(dispatcher.route("session/new"), Route::Request(_)));
        assert!(matches!(dispatcher.route("session/bogus"), Route::NotFound));
        assert_eq!(dispatcher.request_count(), 1);
    }

    #[test]
    fn event_mapping_covers_mapped_and_dropped_kinds() {
        assert_eq!(
            map_event(AgentEventKind::Reasoning),
            EventDispatch::ThoughtChunk
        );
        assert_eq!(map_event(AgentEventKind::Text), EventDispatch::MessageChunk);
        assert_eq!(
            map_event(AgentEventKind::ToolDispatch),
            EventDispatch::ToolCallPending
        );
        assert_eq!(
            map_event(AgentEventKind::ToolResult),
            EventDispatch::ToolCallUpdate
        );
        assert_eq!(
            map_event(AgentEventKind::Notice),
            EventDispatch::WarningChunk
        );
        assert_eq!(
            map_event(AgentEventKind::CompactionDone),
            EventDispatch::CompactionChunk
        );
        assert_eq!(
            map_event(AgentEventKind::ApprovalRequest),
            EventDispatch::PermissionRequest
        );
        assert_eq!(
            map_event(AgentEventKind::AskRequest),
            EventDispatch::AskRoundTrip
        );
        assert_eq!(
            map_event(AgentEventKind::ExtensionSurface),
            EventDispatch::ExtensionDualEmission
        );
        assert_eq!(
            map_event(AgentEventKind::McpSurfaceReady),
            EventDispatch::PublishMcpStatus
        );
        assert_eq!(
            map_event(AgentEventKind::StreamAttempt),
            EventDispatch::BookkeepingOnly
        );

        for kind in [
            AgentEventKind::TurnStarted,
            AgentEventKind::Message,
            AgentEventKind::Usage,
            AgentEventKind::Phase,
            AgentEventKind::TurnDone,
            AgentEventKind::CompactionStarted,
            AgentEventKind::ToolProgress,
            AgentEventKind::Retrying,
            AgentEventKind::Steer,
            AgentEventKind::GuardianAssessment,
            AgentEventKind::ContextMaintenance,
            AgentEventKind::TodoProgress,
            AgentEventKind::WorkspaceLease,
        ] {
            assert_eq!(map_event(kind), EventDispatch::NoUpdate, "{kind:?}");
        }
    }

    #[test]
    fn clip_respects_utf8_boundary_and_suffix() {
        let plain = clip("short result");
        assert_eq!(plain, "short result");

        let long = "x".repeat(MAX_RESULT_CHARS + 10);
        let clipped = clip(&long);
        assert!(clipped.ends_with("10 more chars truncated)"));
        assert_eq!(
            clipped.len(),
            MAX_RESULT_CHARS + "\n…(10 more chars truncated)".len()
        );

        let multibyte = "字".repeat(MAX_RESULT_CHARS / 3 + 5);
        let clipped = clip(&multibyte);
        assert!(std::str::from_utf8(clipped.as_bytes()).is_ok());
        assert!(clipped.ends_with("more chars truncated)"));
    }

    #[test]
    fn diff_passthrough_fence_outruns_body_backticks() {
        let diff = "```diff\n- a\n+ b\n```";
        let out = tool_result_text(diff, true);
        // Body itself carries a three-backtick fence, so the wrapper needs four.
        assert!(out.starts_with("````diff\n"));
        assert!(out.ends_with("\n````"));

        let plain = tool_result_text("hello", false);
        assert_eq!(plain, "hello");
    }

    #[test]
    fn raw_input_keeps_only_valid_json() {
        assert!(raw_input("").is_none());
        assert!(raw_input("{not json").is_none());
        assert_eq!(raw_input(r#"{"path":"a.go"}"#).unwrap()["path"], "a.go");
    }

    #[test]
    fn locations_whitelist_offset_and_absolute_path() {
        let loc = tool_locations("read_file", r#"{"path":"a.go","offset":9}"#, "/proj").unwrap();
        assert_eq!(loc[0].path, "/proj/a.go");
        assert_eq!(loc[0].line, Some(10));

        let search = tool_locations("grep", r#"{"path":"src"}"#, "/proj");
        assert!(search.is_none());

        let edit = tool_locations("edit_file", r#"{"path":"/abs/b.go"}"#, "/proj").unwrap();
        assert_eq!(edit[0].path, "/abs/b.go");
        assert!(tool_locations("read_file", "nope", "/proj").is_none());
        assert!(tool_locations("read_file", r#"{"path":"  "}"#, "/proj").is_none());
    }

    #[test]
    fn tool_kind_explicit_and_heuristic() {
        assert_eq!(tool_kind("read_file"), "read");
        assert_eq!(tool_kind("grep"), "search");
        assert_eq!(tool_kind("write_file"), "edit");
        assert_eq!(tool_kind("bash"), "execute");
        assert_eq!(tool_kind("find_symbols"), "search");
        assert_eq!(tool_kind("replace_text"), "edit");
        assert_eq!(tool_kind("cat_file"), "read");
        assert_eq!(tool_kind("run_tests"), "execute");
        assert_eq!(tool_kind("random_tool"), "other");
    }
}
