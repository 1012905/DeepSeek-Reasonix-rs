//! `sessions` maintenance verbs, their fall-through to the session machine, and
//! `catalogs reindex`: CLI-035, CLI-036. The reindex/diagnose/cleanup store
//! work and the catalog rebuild are unmigrated backend anchors.

use reasonix_cli::text::{CATALOGS_REINDEX_USAGE, CATALOGS_UNKNOWN, SESSIONS_MAINTENANCE_USAGE};
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// The three maintenance verbs land on the sessions-maintenance route.
#[test]
fn sessions_maintenance_verbs_route_through() {
    for verb in ["reindex", "diagnose", "cleanup"] {
        let out = run(&["sessions", verb]);
        assert!(
            matches!(&out.route, Route::SessionsMaintenance { verb: v, .. } if v == verb),
            "{verb}: {out:?}"
        );
    }
}

/// A maintenance verb with an unknown flag prints the maintenance usage, exit 2.
#[test]
fn sessions_maintenance_unknown_flag_usage_exit_two() {
    let out = run(&["sessions", "reindex", "--bogus"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("{SESSIONS_MAINTENANCE_USAGE}\n"));
}

/// Non-maintenance verbs fall through to the session machine interface.
#[test]
fn sessions_non_maintenance_falls_through_to_session_machine() {
    let out = run(&["sessions", "list", "--json"]);
    assert_eq!(out.code, 0);
    assert_eq!(
        out.stdout,
        "{\"schema_version\":1,\"command\":\"session.list\",\"sessions\":[]}"
    );
}

/// `catalogs reindex history|usage` routes to the catalog anchor.
#[test]
fn catalogs_reindex_known_catalogs_route_through() {
    for catalog in ["history", "usage"] {
        let out = run(&["catalogs", "reindex", catalog]);
        assert!(
            matches!(&out.route, Route::Catalogs { catalog: c, .. } if c == catalog),
            "{catalog}: {out:?}"
        );
    }
}

/// An unregistered catalog (e.g. `tasks`) goes to the unknown-catalog
/// diagnostic, exit 2; the reference registers only history/usage.
#[test]
fn catalogs_unknown_catalog_exit_two() {
    let out = run(&["catalogs", "reindex", "tasks"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        format!("{}\n", CATALOGS_UNKNOWN.replace("%s", "tasks"))
    );
}

/// A missing `reindex` target prints the catalog usage, exit 2.
#[test]
fn catalogs_missing_target_usage_exit_two() {
    let cases: &[&[&str]] = &[&["catalogs"], &["catalogs", "reindex"]];
    for argv in cases {
        let out = run(argv);
        assert_eq!(out.code, 2, "{argv:?}");
        assert_eq!(
            out.stderr,
            format!("{CATALOGS_REINDEX_USAGE}\n"),
            "{argv:?}"
        );
    }
}
