//! TUI aliases and flag-set boundary: CLI-018.

use reasonix_cli::flags::RESUME_PICKER_SENTINEL;
use reasonix_cli::{BuildInfo, CliEnv, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str], env: &CliEnv) -> reasonix_cli::CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), env)
}

/// `tui`, `chat`, and `code` are the same TUI route.
#[test]
fn tui_aliases_route_to_tui() {
    for alias in ["tui", "chat", "code"] {
        let out = run(&[alias], &CliEnv::tty());
        assert!(matches!(out.route, Route::Tui { .. }), "{alias}: {out:?}");
    }
}

/// `--inline` registers only under the TUI; it parses there as a bool.
#[test]
fn inline_flag_parses_under_tui() {
    let out = run(&["tui", "--inline"], &CliEnv::tty());
    match out.route {
        Route::Tui { flags, .. } => assert!(flags.bool("inline")),
        other => panic!("expected Tui route, got {other:?}"),
    }
}

/// `--resume`/`-r` take an optional value: a bare `-r` opens the picker
/// (sentinel), a following token joins the flag.
#[test]
fn resume_is_optional_value() {
    let bare = run(&["tui", "-r"], &CliEnv::tty());
    match bare.route {
        Route::Tui { flags, .. } => assert_eq!(flags.value("resume"), Some(RESUME_PICKER_SENTINEL)),
        other => panic!("expected Tui route, got {other:?}"),
    }

    let joined = run(&["tui", "--resume", "abc"], &CliEnv::tty());
    match joined.route {
        Route::Tui { flags, .. } => assert_eq!(flags.value("resume"), Some("abc")),
        other => panic!("expected Tui route, got {other:?}"),
    }
}

/// The TUI has no `--auto`/`-y` approval flag; parsing one fails with the
/// generic flag error and exit 2.
#[test]
fn tui_rejects_auto_flag() {
    for argv in [["tui", "--auto"], ["tui", "-y"]] {
        let out = run(&argv, &CliEnv::tty());
        assert_eq!(out.code, 2, "{argv:?}");
        assert!(
            out.stderr.starts_with("error: unknown"),
            "{argv:?}: {}",
            out.stderr
        );
    }
}

/// A TUI invocation without an interactive terminal is refused with the
/// terminal hint, exit 2.
#[test]
fn tui_outside_console_refused() {
    let out = run(&["tui"], &CliEnv::pipe());
    assert_eq!(out.code, 2);
    assert!(
        out.stderr
            .contains("reasonix tui needs an interactive terminal")
    );
}

/// An unknown TUI flag fails parsing before the terminal guard.
#[test]
fn tui_unknown_flag_exit_two() {
    let out = run(&["tui", "--no-such-flag"], &CliEnv::pipe());
    assert_eq!(out.code, 2);
    assert!(
        out.stderr
            .starts_with("error: unknown flag: --no-such-flag")
    );
}
