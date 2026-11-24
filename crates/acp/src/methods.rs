//! Wire method names, frozen against `internal/frontend/acp`.
//!
//! Standard ACP methods are plain `domain/verb` strings; vendor extensions
//! live under the reserved `_<vendor>/` prefix so they cannot collide with a
//! future official method (protocol.go:553-557).

// ---- Standard ACP methods (service.go:100-125) -----------------------------

pub const INITIALIZE: &str = "initialize";
pub const AUTHENTICATE: &str = "authenticate";

pub const SESSION_NEW: &str = "session/new";
pub const SESSION_LOAD: &str = "session/load";
pub const SESSION_RESUME: &str = "session/resume";
pub const SESSION_PROMPT: &str = "session/prompt";
pub const SESSION_CANCEL: &str = "session/cancel";
pub const SESSION_LIST: &str = "session/list";
pub const SESSION_CLOSE: &str = "session/close";
pub const SESSION_DELETE: &str = "session/delete";
pub const SESSION_SET_CONFIG_OPTION: &str = "session/set_config_option";
pub const SESSION_SET_MODEL: &str = "session/set_model";
pub const SESSION_SET_MODE: &str = "session/set_mode";
pub const SESSION_REQUEST_PERMISSION: &str = "session/request_permission";

pub const FS_READ_TEXT_FILE: &str = "fs/read_text_file";
pub const FS_WRITE_TEXT_FILE: &str = "fs/write_text_file";

pub const TERMINAL_CREATE: &str = "terminal/create";
pub const TERMINAL_WAIT_FOR_EXIT: &str = "terminal/wait_for_exit";
pub const TERMINAL_OUTPUT: &str = "terminal/output";
pub const TERMINAL_KILL: &str = "terminal/kill";
pub const TERMINAL_RELEASE: &str = "terminal/release";

pub const ELICITATION_CREATE: &str = "elicitation/create";

/// JSON-RPC 2.0 cancellation notification (cancel_request.go:16).
pub const CANCEL_REQUEST: &str = "$/cancel_request";

/// Agent → client notification carrying one session/update variant.
pub const SESSION_UPDATE: &str = "session/update";

// ---- Vendor extensions (protocol.go:492-505,557; status.go; session_info.go;
// session_mcp_control.go; session_goal.go) -----------------------------------

pub const SESSION_STEER: &str = "_reasonix.io/session/steer";

pub const SESSION_INBOX_ENQUEUE: &str = "_reasonix.io/session/inbox/enqueue";
pub const SESSION_INBOX_LIST: &str = "_reasonix.io/session/inbox/list";
pub const SESSION_INBOX_GET: &str = "_reasonix.io/session/inbox/get";
pub const SESSION_INBOX_UPDATE: &str = "_reasonix.io/session/inbox/update";
pub const SESSION_INBOX_DELETE: &str = "_reasonix.io/session/inbox/delete";
pub const SESSION_INBOX_MOVE: &str = "_reasonix.io/session/inbox/move";
pub const SESSION_INBOX_SET_PAUSED: &str = "_reasonix.io/session/inbox/setPaused";
pub const SESSION_INBOX_RETRY: &str = "_reasonix.io/session/inbox/retry";
pub const SESSION_INBOX_REFRESH: &str = "_reasonix.io/session/inbox/refresh";

pub const SESSION_RELOAD_EXTENSIONS: &str = "_reasonix.io/session/reloadExtensions";

pub const SESSION_STATUS: &str = "_reasonix.io/session/status";
pub const SESSION_STATUS_UPDATE: &str = "_reasonix.io/session/status_update";
pub const SESSION_MCP_STATUS: &str = "_reasonix.io/session/mcpStatus";
pub const SESSION_MCP_STATUS_UPDATE: &str = "_reasonix.io/session/mcpStatus_update";
pub const SESSION_INFO: &str = "_reasonix.io/session/info";
pub const SESSION_MCP_SERVERS: &str = "_reasonix.io/session/mcpServers";
pub const SESSION_MCP_RECONNECT: &str = "_reasonix.io/session/mcp/reconnect";
pub const SESSION_MCP_DISCONNECT: &str = "_reasonix.io/session/mcp/disconnect";
pub const SESSION_EXTENSION_SUBMIT_FORM: &str = "_reasonix.io/session/extension/submitForm";

pub const SESSION_GOAL_RESUME: &str = "_reasonix.io/session/goal/resume";
pub const SESSION_GOAL_PAUSE: &str = "_reasonix.io/session/goal/pause";

/// The nine inbox verbs and their method names, in the initialize capability
/// order advertised by `sessionInbox.methods` (handshake.go:41-51).
pub const INBOX_METHODS: &[(&str, &str)] = &[
    ("enqueue", SESSION_INBOX_ENQUEUE),
    ("list", SESSION_INBOX_LIST),
    ("get", SESSION_INBOX_GET),
    ("update", SESSION_INBOX_UPDATE),
    ("delete", SESSION_INBOX_DELETE),
    ("move", SESSION_INBOX_MOVE),
    ("setPaused", SESSION_INBOX_SET_PAUSED),
    ("retry", SESSION_INBOX_RETRY),
    ("refresh", SESSION_INBOX_REFRESH),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbox_methods_cover_nine_verbs() {
        assert_eq!(INBOX_METHODS.len(), 9);
        for (verb, method) in INBOX_METHODS {
            assert!(method.starts_with("_reasonix.io/session/inbox/"));
            assert!(method.ends_with(verb));
        }
    }
}
