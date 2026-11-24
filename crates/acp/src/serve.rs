//! Serve registry: the inbound request and notification method table
//! (service.go:100-125; matrix B §4).
//!
//! The reference registers 32 request handlers through `Conn.Handle` and one
//! notification handler, `session/cancel`, through `Conn.HandleNotify`. This
//! module freezes that table in registration order and registers stub handlers
//! onto the pure [`Dispatcher`]; real handlers are session-runtime wiring. An
//! unregistered request falls through to `-32601 method not found: <method>`;
//! an unregistered notification is silently dropped.

use crate::dispatch::{Dispatcher, NotificationHandler, RequestHandler};
use crate::errors::RpcError;
use crate::methods::{
    AUTHENTICATE, INITIALIZE, SESSION_CANCEL, SESSION_CLOSE, SESSION_DELETE,
    SESSION_EXTENSION_SUBMIT_FORM, SESSION_GOAL_PAUSE, SESSION_GOAL_RESUME, SESSION_INBOX_DELETE,
    SESSION_INBOX_ENQUEUE, SESSION_INBOX_GET, SESSION_INBOX_LIST, SESSION_INBOX_MOVE,
    SESSION_INBOX_REFRESH, SESSION_INBOX_RETRY, SESSION_INBOX_SET_PAUSED, SESSION_INBOX_UPDATE,
    SESSION_INFO, SESSION_LIST, SESSION_LOAD, SESSION_MCP_DISCONNECT, SESSION_MCP_RECONNECT,
    SESSION_MCP_SERVERS, SESSION_MCP_STATUS, SESSION_NEW, SESSION_PROMPT,
    SESSION_RELOAD_EXTENSIONS, SESSION_RESUME, SESSION_SET_CONFIG_OPTION, SESSION_SET_MODE,
    SESSION_SET_MODEL, SESSION_STATUS, SESSION_STEER,
};
use serde_json::Value;

/// Inbound request methods in reference registration order (matrix B §4.2).
pub const SERVE_REQUEST_METHODS: &[&str] = &[
    INITIALIZE,
    AUTHENTICATE,
    SESSION_NEW,
    SESSION_LOAD,
    SESSION_RESUME,
    SESSION_PROMPT,
    SESSION_STEER,
    SESSION_INBOX_ENQUEUE,
    SESSION_INBOX_LIST,
    SESSION_INBOX_GET,
    SESSION_INBOX_UPDATE,
    SESSION_INBOX_DELETE,
    SESSION_INBOX_MOVE,
    SESSION_INBOX_SET_PAUSED,
    SESSION_INBOX_RETRY,
    SESSION_INBOX_REFRESH,
    SESSION_RELOAD_EXTENSIONS,
    SESSION_GOAL_RESUME,
    SESSION_GOAL_PAUSE,
    SESSION_STATUS,
    SESSION_MCP_STATUS,
    SESSION_INFO,
    SESSION_MCP_SERVERS,
    SESSION_MCP_RECONNECT,
    SESSION_MCP_DISCONNECT,
    SESSION_EXTENSION_SUBMIT_FORM,
    SESSION_SET_CONFIG_OPTION,
    SESSION_SET_MODEL,
    SESSION_SET_MODE,
    SESSION_CLOSE,
    SESSION_LIST,
    SESSION_DELETE,
];

/// Inbound notification methods; the only one the reference registers.
pub const SERVE_NOTIFICATION_METHODS: &[&str] = &[SESSION_CANCEL];

/// Stub request handler: returns a null result. The runtime replaces these with
/// the real session handlers at connection setup.
struct StubRequest;
impl RequestHandler for StubRequest {
    fn call(&self, _params: Option<Value>) -> Result<Value, RpcError> {
        Ok(Value::Null)
    }
}

/// Stub notification handler: fire and forget.
struct StubNotification;
impl NotificationHandler for StubNotification {
    fn call(&self, _params: Option<Value>) {}
}

/// Registers the full reference table onto a dispatcher with stub handlers.
pub fn register_default(dispatcher: &mut Dispatcher) {
    for method in SERVE_REQUEST_METHODS {
        dispatcher.register_request(method, Box::new(StubRequest));
    }
    for method in SERVE_NOTIFICATION_METHODS {
        dispatcher.register_notification(method, Box::new(StubNotification));
    }
}

/// True when `method` is a registered inbound request.
pub fn is_registered_request(method: &str) -> bool {
    SERVE_REQUEST_METHODS.contains(&method)
}

/// True when `method` is a registered inbound notification.
pub fn is_registered_notification(method: &str) -> bool {
    SERVE_NOTIFICATION_METHODS.contains(&method)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_lists_all_thirty_two_requests_and_one_notification() {
        assert_eq!(SERVE_REQUEST_METHODS.len(), 32);
        assert_eq!(SERVE_NOTIFICATION_METHODS.len(), 1);
        assert_eq!(SERVE_NOTIFICATION_METHODS[0], SESSION_CANCEL);
    }

    #[test]
    fn dispatcher_routes_every_registered_method() {
        let mut dispatcher = Dispatcher::default();
        register_default(&mut dispatcher);
        for method in SERVE_REQUEST_METHODS {
            assert!(
                matches!(dispatcher.route(method), crate::dispatch::Route::Request(_)),
                "{method} not routed as request"
            );
        }
        for method in SERVE_NOTIFICATION_METHODS {
            assert!(
                matches!(
                    dispatcher.route(method),
                    crate::dispatch::Route::Notification(_)
                ),
                "{method} not routed as notification"
            );
        }
        assert!(matches!(
            dispatcher.route("session/bogus"),
            crate::dispatch::Route::NotFound
        ));
        assert_eq!(dispatcher.request_count(), 32);
        assert_eq!(dispatcher.notification_count(), 1);
    }

    #[test]
    fn unknown_request_is_not_found_and_unknown_notification_is_dropped() {
        assert!(!is_registered_request("session/bogus"));
        assert!(!is_registered_notification("session/bogus"));
        assert!(is_registered_request(SESSION_NEW));
        assert!(is_registered_notification(SESSION_CANCEL));
    }
}
