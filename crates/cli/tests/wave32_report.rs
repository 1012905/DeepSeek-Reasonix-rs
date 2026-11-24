//! `report` action matrix and its usage guards: CLI-040. The report store and
//! upload flow are unmigrated anchors.

use reasonix_cli::text::REPORT_USAGE_BODY;
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// More than two arguments, an unknown action, and an ID on `list` all print
/// the report usage to stderr, exit 2.
#[test]
fn report_usage_guards_exit_two() {
    let cases: &[&[&str]] = &[
        &["report", "show", "id", "extra"],
        &["report", "frobnicate"],
        &["report", "list", "id"],
    ];
    for argv in cases {
        let out = run(argv);
        assert_eq!(out.stderr, format!("{REPORT_USAGE_BODY}\n"), "{argv:?}");
        assert_eq!(out.code, 2, "{argv:?}");
    }
}

/// Bare `report` previews the newest local report: the unmigrated anchor.
#[test]
fn report_bare_routes_through() {
    let out = run(&["report"]);
    assert!(matches!(out.route, Route::Verb { command, .. } if command == "report"));
}

/// Known actions route through with their optional ID.
#[test]
fn report_known_actions_route_through() {
    let cases: &[&[&str]] = &[
        &["report", "list"],
        &["report", "show", "id"],
        &["report", "send", "id"],
        &["report", "delete", "id"],
    ];
    for argv in cases {
        let out = run(argv);
        assert!(
            matches!(out.route, Route::Verb { command, .. } if command == "report"),
            "{argv:?}"
        );
    }
}
