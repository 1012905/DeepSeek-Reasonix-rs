//! `subagent` verb matrix, case folding, and usage stream ownership: CLI-031.

use reasonix_cli::text::SUBAGENT_USAGE;
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// A bare `subagent` prints its usage to stderr, exit 2.
#[test]
fn subagent_bare_usage_stderr_exit_two() {
    let out = run(&["subagent"]);
    assert!(out.stdout.is_empty());
    assert_eq!(out.stderr, format!("{SUBAGENT_USAGE}\n"));
    assert_eq!(out.code, 2);
}

/// `subagent help` prints the usage to stdout, exit 0.
#[test]
fn subagent_help_usage_stdout_exit_zero() {
    let out = run(&["subagent", "help"]);
    assert_eq!(out.stdout, format!("{SUBAGENT_USAGE}\n"));
    assert_eq!(out.code, 0);
}

/// An unknown command prints diagnostic plus usage to stderr, exit 2.
#[test]
fn subagent_unknown_command_exit_two() {
    let out = run(&["subagent", "frobnicate"]);
    assert!(out.stdout.is_empty());
    assert_eq!(
        out.stderr,
        format!("unknown subagent command \"frobnicate\"\n\n{SUBAGENT_USAGE}\n")
    );
    assert_eq!(out.code, 2);
}

/// Verb names are case-insensitive; mixed-case spellings fold to the known
/// route.
#[test]
fn subagent_verbs_fold_case() {
    for verb in ["LIST", "New", "Edit", "DELETE", "Try", "RUN"] {
        let out = run(&["subagent", verb]);
        match &out.route {
            Route::Verb {
                command,
                verb: routed,
                ..
            } => {
                assert_eq!(command, "subagent");
                assert_eq!(routed, &verb.to_lowercase(), "{verb}");
            }
            other => panic!("expected subagent route for {verb}, got {other:?}"),
        }
    }
}
