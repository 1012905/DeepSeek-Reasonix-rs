//! Dispatch table and unknown-command fallback: CLI-009, CLI-015.

use reasonix_cli::text::USAGE_BODY;
use reasonix_cli::{BuildInfo, CliEnv, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> reasonix_cli::CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// CLI-009: every verb and alias in the reference dispatch table resolves to a
/// known route; the table itself is decidable even where the handler has not
/// migrated yet.
#[test]
fn dispatch_table_routes_known_verbs() {
    let known = [
        "run",
        "serve",
        "tui",
        "chat",
        "code",
        "web",
        "setup",
        "config",
        "init",
        "acp",
        "mcp",
        "login",
        "whoami",
        "logout",
        "remote",
        "plugin",
        "subagent",
        "doctor",
        "report",
        "session",
        "sessions",
        "catalogs",
        "hook",
        "hooks",
        "trust",
        "task",
        "review",
        "upgrade",
        "update",
        "version",
        "completion",
        "docs-manifest",
        "help",
        "--version",
        "-v",
        "--help",
        "-h",
        "schedule",
        "sandbox",
    ];
    for verb in known {
        let out = run(&[verb]);
        assert!(
            !matches!(out.route, Route::Unknown { .. }),
            "{verb} routed to Unknown: {out:?}"
        );
    }
}

/// CLI-015: an unknown command names itself on stderr, prints the usage body to
/// stdout, and exits 2.
#[test]
fn unknown_command_stderr_usage_exit_two() {
    let out = run(&["nosuchcommand"]);
    assert_eq!(out.stdout, USAGE_BODY);
    assert_eq!(out.stderr, "unknown command \"nosuchcommand\"\n\n");
    assert_eq!(out.code, 2);
    assert_eq!(
        out.route,
        Route::Unknown {
            command: "nosuchcommand".to_owned()
        }
    );
}

/// CLI-015: the hidden `schedule` and `sandbox` verbs are intercepted before the
/// unknown-command fallback.
#[test]
fn hidden_verbs_are_not_unknown() {
    let schedule = run(&["schedule", "list"]);
    assert!(matches!(schedule.route, Route::Schedule { .. }));
    let sandbox = run(&["sandbox", "list"]);
    assert!(matches!(sandbox.route, Route::Sandbox { .. }));
}
