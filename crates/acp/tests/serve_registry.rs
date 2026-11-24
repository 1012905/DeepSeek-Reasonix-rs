//! ACP-022: serve registry — every registered request method enumerated, the
//! single notification, and the unknown-method fall-through.

use reasonix_acp::dispatch::Dispatcher;
use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::serve::{
    SERVE_NOTIFICATION_METHODS, SERVE_REQUEST_METHODS, is_registered_notification,
    is_registered_request, register_default,
};

/// The 32 inbound request methods in reference registration order
/// (service.go:100-125; matrix B §4.2).
const EXPECTED_REQUESTS: [&str; 32] = [
    "initialize",
    "authenticate",
    "session/new",
    "session/load",
    "session/resume",
    "session/prompt",
    "_reasonix.io/session/steer",
    "_reasonix.io/session/inbox/enqueue",
    "_reasonix.io/session/inbox/list",
    "_reasonix.io/session/inbox/get",
    "_reasonix.io/session/inbox/update",
    "_reasonix.io/session/inbox/delete",
    "_reasonix.io/session/inbox/move",
    "_reasonix.io/session/inbox/setPaused",
    "_reasonix.io/session/inbox/retry",
    "_reasonix.io/session/inbox/refresh",
    "_reasonix.io/session/reloadExtensions",
    "_reasonix.io/session/goal/resume",
    "_reasonix.io/session/goal/pause",
    "_reasonix.io/session/status",
    "_reasonix.io/session/mcpStatus",
    "_reasonix.io/session/info",
    "_reasonix.io/session/mcpServers",
    "_reasonix.io/session/mcp/reconnect",
    "_reasonix.io/session/mcp/disconnect",
    "_reasonix.io/session/extension/submitForm",
    "session/set_config_option",
    "session/set_model",
    "session/set_mode",
    "session/close",
    "session/list",
    "session/delete",
];

#[test]
fn registry_table_matches_every_reference_request() {
    assert_eq!(SERVE_REQUEST_METHODS.len(), EXPECTED_REQUESTS.len());
    for (registered, expected) in SERVE_REQUEST_METHODS.iter().zip(EXPECTED_REQUESTS.iter()) {
        assert_eq!(registered, expected, "registry order drifted");
    }
    assert_eq!(SERVE_NOTIFICATION_METHODS, ["session/cancel"]);
}

#[test]
fn dispatcher_routes_every_registered_method_and_drops_unknown() {
    let mut dispatcher = Dispatcher::default();
    register_default(&mut dispatcher);

    for method in EXPECTED_REQUESTS {
        assert!(
            matches!(
                dispatcher.route(method),
                reasonix_acp::dispatch::Route::Request(_)
            ),
            "{method} not routed as request"
        );
        assert!(is_registered_request(method));
    }
    for method in SERVE_NOTIFICATION_METHODS {
        assert!(
            matches!(
                dispatcher.route(method),
                reasonix_acp::dispatch::Route::Notification(_)
            ),
            "{method} not routed as notification"
        );
        assert!(is_registered_notification(method));
    }

    assert!(matches!(
        dispatcher.route("session/bogus"),
        reasonix_acp::dispatch::Route::NotFound
    ));
    assert!(!is_registered_request("session/bogus"));
    assert!(!is_registered_notification("session/bogus"));
    assert_eq!(dispatcher.request_count(), 32);
    assert_eq!(dispatcher.notification_count(), 1);
}

#[test]
fn unknown_request_answers_method_not_found_identity() {
    let err = reasonix_acp::errors::RpcError::method_not_found("session/bogus");
    assert_eq!(err.code, RpcErrorCode::MethodNotFound);
    assert_eq!(err.code.code(), -32601);
    assert_eq!(err.message, "method not found: session/bogus");
}
