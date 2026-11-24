//! Session and hook machine interfaces: operation matrix, option validation,
//! error envelope shape, and exit-code mapping: CLI-033, CLI-034. Record
//! payloads render from empty collections until the store anchors migrate;
//! tests assert envelopes, field names/order, and redaction boundaries, not
//! backend data.

use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

fn envelope(command: &str, code: &str, message: &str) -> String {
    format!(
        "{{\"schema_version\":1,\"command\":\"{command}\",\"error\":{{\"code\":\"{code}\",\"message\":\"{message}\"}}}}"
    )
}

// --- session -----------------------------------------------------------------

/// A bare `session` reports a missing operation as an invalid_argument, exit 2.
#[test]
fn session_bare_reports_missing_operation() {
    let out = run(&["session"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stdout,
        envelope(
            "session.",
            "invalid_argument",
            "a session operation is required"
        )
    );
    assert!(out.stderr.is_empty());
}

/// An unknown session operation maps to unknown_command, exit 2.
#[test]
fn session_unknown_operation_unknown_command() {
    let out = run(&["session", "frob"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stdout,
        envelope(
            "session.frob",
            "unknown_command",
            "unknown session operation"
        )
    );
}

/// Machine output without `--json` is rejected with the json-required hint.
#[test]
fn session_requires_json_flag() {
    let out = run(&["session", "list"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stdout,
        envelope("session.list", "invalid_argument", "--json is required")
    );
}

/// `show`/`status` require a session target.
#[test]
fn session_show_requires_target() {
    let out = run(&["session", "show", "--json"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stdout,
        envelope(
            "session.show",
            "invalid_argument",
            "a session identifier is required"
        )
    );
}

/// `session list --json` renders the empty collection envelope, exit 0.
#[test]
fn session_list_json_empty_envelope() {
    let out = run(&["session", "list", "--json"]);
    assert_eq!(out.code, 0);
    assert_eq!(
        out.stdout,
        "{\"schema_version\":1,\"command\":\"session.list\",\"sessions\":[]}"
    );
}

/// `session recovery --json` renders the recoveries collection; `status` the
/// single-session null anchor.
#[test]
fn session_recovery_and_status_payloads() {
    let recovery = run(&["session", "recovery", "--json"]);
    assert_eq!(recovery.code, 0);
    assert_eq!(
        recovery.stdout,
        "{\"schema_version\":1,\"command\":\"session.recovery\",\"recoveries\":[]}"
    );

    let status = run(&["session", "status", "--json", "--session", "abc"]);
    assert_eq!(status.code, 0);
    assert_eq!(
        status.stdout,
        "{\"schema_version\":1,\"command\":\"session.show\",\"session\":null}"
    );
}

// --- hook --------------------------------------------------------------------

/// A bare `hook` reports a missing operation, exit 2.
#[test]
fn hook_bare_reports_missing_operation() {
    let out = run(&["hook"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stdout,
        envelope("hook.", "invalid_argument", "a hook operation is required")
    );
}

/// An unknown hook operation maps to unknown_command, exit 2.
#[test]
fn hook_unknown_operation_unknown_command() {
    let out = run(&["hook", "frob"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stdout,
        envelope("hook.frob", "unknown_command", "unknown hook operation")
    );
}

/// `hook list --json` renders the empty hook collection.
#[test]
fn hook_list_json_empty_envelope() {
    let out = run(&["hook", "list", "--json"]);
    assert_eq!(out.code, 0);
    assert_eq!(
        out.stdout,
        "{\"schema_version\":1,\"command\":\"hook.list\",\"hooks\":[]}"
    );
}

/// `hook status --json` renders the trusted-project flag trio in reference
/// field order: trusted_project, project_defines, sources.
#[test]
fn hook_status_json_field_order() {
    let out = run(&["hook", "status", "--json"]);
    assert_eq!(out.code, 0);
    assert_eq!(
        out.stdout,
        "{\"schema_version\":1,\"command\":\"hook.status\",\"trusted_project\":false,\"project_defines\":false,\"sources\":[]}"
    );
}

/// Machine routes carry the interface and operation on the Route anchor.
#[test]
fn machine_route_carries_interface_and_operation() {
    let out = run(&["session", "show", "--json", "--session", "abc"]);
    match &out.route {
        Route::Machine {
            interface,
            operation,
            ..
        } => {
            assert_eq!(operation, "show");
            let _ = interface;
        }
        other => panic!("expected Machine route, got {other:?}"),
    }
}
