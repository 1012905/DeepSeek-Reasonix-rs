//! Shell completion scripts and the hidden `__complete` protocol: CLI-041.

use reasonix_cli::text::COMPLETION_USAGE;
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// Bare `completion` prints the usage to stdout, exit 2.
#[test]
fn completion_bare_usage_stdout_exit_two() {
    let out = run(&["completion"]);
    assert_eq!(out.stdout, format!("{COMPLETION_USAGE}\n"));
    assert!(out.stderr.is_empty());
    assert_eq!(out.code, 2);
}

/// `completion help` prints the usage to stdout, exit 0.
#[test]
fn completion_help_usage_stdout_exit_zero() {
    let out = run(&["completion", "--help"]);
    assert_eq!(out.stdout, format!("{COMPLETION_USAGE}\n"));
    assert_eq!(out.code, 0);
}

/// An unknown shell names itself in the stderr diagnostic, exit 2.
#[test]
fn completion_unknown_shell_exit_two() {
    let out = run(&["completion", "fishx"]);
    assert!(out.stdout.is_empty());
    assert_eq!(
        out.stderr,
        "unknown shell \"fishx\" (want bash, zsh, or fish)\n"
    );
    assert_eq!(out.code, 2);
}

/// Each known shell emits its script to stdout, exit 0, delegating to the
/// `__complete` protocol with the current cword and words.
#[test]
fn completion_scripts_delegate_to_complete_protocol() {
    for shell in ["bash", "zsh", "fish"] {
        let out = run(&["completion", shell]);
        assert_eq!(out.code, 0, "{shell}");
        assert!(out.stdout.contains("completion __complete"), "{shell}");
        assert!(out.stderr.is_empty(), "{shell}");
        match &out.route {
            Route::Completion { shell: routed } => assert_eq!(routed, shell),
            other => panic!("expected Completion route, got {other:?}"),
        }
    }
}

/// The `__complete` protocol rejects a non-integer cword, exit 2.
#[test]
fn complete_protocol_rejects_non_integer_cword() {
    let out = run(&["completion", "__complete", "abc", "reasonix"]);
    assert_eq!(out.code, 2);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
}

/// The `__complete` protocol rejects a negative cword, exit 2.
#[test]
fn complete_protocol_rejects_negative_cword() {
    let out = run(&["completion", "__complete", "-1", "reasonix"]);
    assert_eq!(out.code, 2);
}

/// A valid cword parses; the candidate stream is the unmigrated backend anchor.
#[test]
fn complete_protocol_valid_cword_routes_through() {
    let out = run(&["completion", "__complete", "1", "reasonix", "run"]);
    assert_eq!(out.code, 0);
}
