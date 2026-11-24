//! Exit-code anchors for the wave 32 surface: CLI-021..050.
//!
//! Decidable here: 0 success/help/empty result, 2 usage/parse/validation,
//! 77 non-human agent-child refusals (covered in the schedule/remote tests), and
//! the unmigrated kernel anchor. Exit 4 (untrusted folder, `run_verdict.go`)
//! and 130 (login Ctrl-C, `account.go:79-82`) have no trigger path in this
//! parse layer; their typed anchors live with the backend handlers.

use reasonix_cli::text::{INIT_HINT, LOGIN_CANCELLED};
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// `init` prints its in-session hint to stdout, exit 0.
#[test]
fn init_hint_exit_zero() {
    let out = run(&["init"]);
    assert_eq!(out.stdout, format!("{INIT_HINT}\n"));
    assert!(out.stderr.is_empty());
    assert_eq!(out.code, 0);
}

/// An unknown command exits 2 with the usage body on stdout.
#[test]
fn unknown_command_exit_two() {
    let out = run(&["no-such-command"]);
    assert_eq!(out.code, 2);
}

/// No wave-32 parse-layer outcome exits 4 (untrusted folder) or 130
/// (login cancel); those are backend anchors, asserted here so a parse-layer
/// regression cannot silently take them over.
#[test]
fn parse_layer_does_not_take_reserved_exit_codes() {
    let cases: &[&[&str]] = &[
        &["run", "task"],
        &["serve"],
        &["acp"],
        &["mcp", "list"],
        &["plugin", "list"],
        &["subagent", "list"],
        &["doctor"],
        &["config", "telemetry"],
        &["report", "list"],
        &["session", "list", "--json"],
        &["hook", "status", "--json"],
        &["task", "list"],
        &["trust"],
        &["review"],
        &["upgrade", "--check"],
        &["setup"],
        &["login"],
        &["remote", "list"],
    ];
    for argv in cases {
        let out = run(argv);
        assert_ne!(out.code, 4, "{argv:?}: {out:?}");
        assert_ne!(out.code, 130, "{argv:?}: {out:?}");
    }
}

/// The login cancelled message is the backend's 130 anchor; its frozen text
/// stays byte-stable even though the Ctrl-C path has not migrated.
#[test]
fn login_cancelled_message_anchor_text() {
    assert_eq!(LOGIN_CANCELLED, "\nlogin: cancelled.");
}
