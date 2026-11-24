//! `plugin` verb matrix, stream ownership, and install guards: CLI-028.

use reasonix_cli::text::{
    PLUGIN_INSTALL_CONFIRM, PLUGIN_NAME_VALUE, PLUGIN_ONE_SOURCE, PLUGIN_USAGE,
};
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// A bare `plugin` prints its usage to stderr and exits 0 (plugin.go:23-26).
#[test]
fn plugin_bare_usage_stderr_exit_zero() {
    let out = run(&["plugin"]);
    assert!(out.stdout.is_empty());
    assert_eq!(out.stderr, format!("{PLUGIN_USAGE}\n"));
    assert_eq!(out.code, 0);
}

/// `plugin help` also prints the usage to stderr, exit 0.
#[test]
fn plugin_help_usage_stderr_exit_zero() {
    let out = run(&["plugin", "--help"]);
    assert!(out.stdout.is_empty());
    assert_eq!(out.stderr, format!("{PLUGIN_USAGE}\n"));
    assert_eq!(out.code, 0);
}

/// An unknown command prints the diagnostic plus the usage to stderr, exit 2.
#[test]
fn plugin_unknown_command_exit_two() {
    let out = run(&["plugin", "frobnicate"]);
    assert!(out.stdout.is_empty());
    assert_eq!(
        out.stderr,
        format!("unknown plugin command \"frobnicate\"\n\n{PLUGIN_USAGE}\n")
    );
    assert_eq!(out.code, 2);
}

/// Install needs exactly one source; zero or two both fail with the guard text.
#[test]
fn plugin_install_requires_one_source() {
    let zero = run(&["plugin", "install", "--dry-run"]);
    assert_eq!(zero.code, 2);
    assert_eq!(zero.stderr, format!("{PLUGIN_ONE_SOURCE}\n"));

    let two = run(&["plugin", "install", "--yes", "a", "b"]);
    assert_eq!(two.code, 2);
    assert_eq!(two.stderr, format!("{PLUGIN_ONE_SOURCE}\n"));
}

/// A source without `--yes` or `--dry-run` is not applied: the confirmation
/// guard fires, exit 2.
#[test]
fn plugin_install_requires_confirmation() {
    let out = run(&["plugin", "install", "source.tar.gz"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("{PLUGIN_INSTALL_CONFIRM}\n"));
}

/// `--name` requires a value.
#[test]
fn plugin_install_name_requires_value() {
    let out = run(&["plugin", "install", "--yes", "--name"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("{PLUGIN_NAME_VALUE}\n"));
}

/// An unknown install flag is rejected by name.
#[test]
fn plugin_install_unknown_flag() {
    let out = run(&["plugin", "install", "--yes", "--bogus", "src"]);
    assert_eq!(out.code, 2);
    assert!(
        out.stderr
            .starts_with("unknown plugin install flag \"--bogus\"")
    );
}

/// A dry-run install with one source reaches the install anchor.
#[test]
fn plugin_install_dry_run_routes_through() {
    let out = run(&["plugin", "install", "--dry-run", "src"]);
    assert!(matches!(
        out.route,
        Route::Verb { command, verb, .. } if command == "plugin" && verb == "install"
    ));
}
