//! `doctor` verb matrix and the repair/catalogs positional guards: CLI-038.
//! The collect/inspect/repair backends are unmigrated anchors.

use reasonix_cli::text::{DOCTOR_CATALOGS_USAGE, DOCTOR_REPAIR_USAGE};
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// Bare `doctor` routes to the collect anchor.
#[test]
fn doctor_bare_routes_to_collect_anchor() {
    let out = run(&["doctor"]);
    assert!(matches!(out.route, Route::Verb { command, .. } if command == "doctor"));
}

/// `repair` takes no positional argument.
#[test]
fn doctor_repair_rejects_positional() {
    let out = run(&["doctor", "repair", "extra"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("{DOCTOR_REPAIR_USAGE}\n"));
}

/// `catalogs` takes no positional argument.
#[test]
fn doctor_catalogs_rejects_positional() {
    let out = run(&["doctor", "catalogs", "extra"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("{DOCTOR_CATALOGS_USAGE}\n"));
}

/// Known diagnostic verbs route through without a parse-layer rejection.
#[test]
fn doctor_known_verbs_route_through() {
    for verb in [
        "quality",
        "session",
        "redact-sessions",
        "capabilities",
        "runtime",
        "billing",
        "repair",
        "catalogs",
    ] {
        let out = run(&["doctor", verb]);
        assert!(
            !matches!(out.route, Route::Unknown { .. }),
            "{verb}: {out:?}"
        );
    }
}
