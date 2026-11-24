//! Bare invocation and top-level help: CLI-007, CLI-008, CLI-008a, CLI-010.

use reasonix_cli::text::{MINTTY_CONSOLE_HINT, STANDALONE_CONSOLE_HINT, USAGE_BODY};
use reasonix_cli::{BuildInfo, CliEnv, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(args: &[&str], env: &CliEnv) -> reasonix_cli::CliOutcome {
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), env)
}

/// CLI-007: `help`, `--help`, and `-h` print the full usage body to stdout.
#[test]
fn help_forms_print_usage_to_stdout_zero() {
    for argv in [["help"], ["--help"], ["-h"]] {
        let out = run(&argv, &CliEnv::pipe());
        assert_eq!(out.stdout, USAGE_BODY, "{}", argv[0]);
        assert!(out.stderr.is_empty(), "{}: {}", argv[0], out.stderr);
        assert_eq!(out.code, 0, "{}", argv[0]);
        assert_eq!(out.route, Route::Help);
    }
}

/// CLI-007: extra positional tokens after `help` are ignored.
#[test]
fn help_ignores_extra_positionals() {
    let out = run(&["help", "whatever"], &CliEnv::pipe());
    assert_eq!(out.stdout, USAGE_BODY);
    assert_eq!(out.code, 0);
}

/// CLI-008: a bare non-TTY invocation prints the usage body and exits 0.
#[test]
fn bare_outside_console_usage_exit_zero() {
    let out = run(&[], &CliEnv::pipe());
    assert_eq!(out.stdout, USAGE_BODY);
    assert!(out.stderr.is_empty());
    assert_eq!(out.code, 0);
    assert_eq!(out.route, Route::BareUsage { bare: true });
}

/// CLI-008a: mintty / Cygwin pty replaces the usage body with the mintty hint.
#[test]
fn mintty_hint_exit_two_without_usage() {
    let mut env = CliEnv::pipe();
    env.mintty = true;
    let out = run(&[], &env);
    assert!(out.stdout.is_empty(), "stdout: {}", out.stdout);
    assert_eq!(out.stderr, MINTTY_CONSOLE_HINT);
    assert_eq!(out.code, 2);
    assert_eq!(out.route, Route::BareMintty);
}

/// CLI-010: both streams are terminals, so the bare invocation routes to the
/// TUI instead of printing usage.
#[test]
fn bare_tty_routes_to_tui() {
    let out = run(&[], &CliEnv::tty());
    assert_eq!(out.route, Route::BareTui);
    assert_eq!(out.code, 2);
}

/// CLI-010: session flags with no console are a usage error (exit 2); the bare
/// no-argument case stays exit 0.
#[test]
fn bare_session_flags_outside_console_exit_two() {
    let out = run(&["--model", "x"], &CliEnv::pipe());
    assert_eq!(out.stdout, USAGE_BODY);
    assert!(out.stderr.is_empty());
    assert_eq!(out.code, 2);
    assert_eq!(out.route, Route::BareUsage { bare: false });
}

/// CLI-010: a console owned alone (Windows double-click) appends the standalone
/// hint after the usage body; the bare exit code stays 0.
#[test]
fn standalone_console_hint_appended_after_usage() {
    let mut env = CliEnv::pipe();
    env.owns_console_alone = true;
    let out = run(&[], &env);
    let mut expected = USAGE_BODY.to_owned();
    expected.push('\n');
    expected.push_str(STANDALONE_CONSOLE_HINT);
    assert_eq!(out.stdout, expected);
    assert_eq!(out.code, 0);
}
