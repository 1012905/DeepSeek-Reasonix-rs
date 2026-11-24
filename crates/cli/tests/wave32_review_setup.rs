//! `review` flag parsing and `setup` help/unknown-flag boundary: CLI-044,
//! CLI-045. The git diff, provider call, and wizard overwrite guards are
//! unmigrated backend anchors.

use reasonix_cli::text::SETUP_USAGE;
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// An unknown review flag fails parsing, exit 2.
#[test]
fn review_unknown_flag_exit_two() {
    let out = run(&["review", "--bogus"]);
    assert_eq!(out.code, 2);
    assert!(out.stderr.starts_with("error: unknown flag: --bogus"));
}

/// `review --help` is a clean route, exit 0.
#[test]
fn review_help_exit_zero() {
    let out = run(&["review", "--help"]);
    assert_eq!(out.code, 0);
    assert!(matches!(out.route, Route::Verb { command, .. } if command == "review"));
}

/// `setup --help` prints the setup usage to stdout, exit 0.
#[test]
fn setup_help_usage_stdout_exit_zero() {
    let out = run(&["setup", "--help"]);
    assert_eq!(out.stdout, format!("{SETUP_USAGE}\n"));
    assert!(out.stderr.is_empty());
    assert_eq!(out.code, 0);
}

/// An unknown setup flag names itself in the stderr diagnostic, exit 2.
#[test]
fn setup_unknown_flag_exit_two() {
    let out = run(&["setup", "--bogus"]);
    assert_eq!(out.code, 2);
    assert!(out.stderr.starts_with("unknown setup flag \"--bogus\""));
    assert!(out.stderr.contains(&format!("{SETUP_USAGE}\n")));
}

/// A clean setup argv reaches the wizard anchor.
#[test]
fn setup_clean_argv_routes_through() {
    let out = run(&["setup", "--local", "config.toml"]);
    assert!(matches!(out.route, Route::Verb { command, .. } if command == "setup"));
}
