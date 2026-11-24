//! Wave 32 preset/profile resolution and the `--copy` session guard:
//! CLI-021, CLI-024. Approval-flag pairwise conflicts are already covered by
//! `run.rs::run_approval_conflicts_exit_two`; this file pins the preset matrix,
//! the deprecation warning, and the copy-requirement exit split.

use reasonix_cli::text::{COPY_REQUIRES_RESUME, PROFILE_DEPRECATED_WARNING};
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// Accepted `--preset` spellings route through to the run route with the raw
/// value preserved for the backend anchor.
#[test]
fn run_preset_matrix_parses_all_aliases() {
    for preset in [
        "balanced", "full", "economy", "light", "lite", "eco", "delivery", "deliver", "quality",
    ] {
        let out = run(&["run", "--preset", preset, "task"]);
        match &out.route {
            Route::Run { flags, .. } => assert_eq!(flags.value("preset"), Some(preset), "{preset}"),
            other => panic!(
                "expected Run route for {preset}, got {other:?}: {}",
                out.stderr
            ),
        }
    }
}

/// An unknown preset fails with the reference quoted error, exit 2.
#[test]
fn run_rejects_unknown_preset() {
    let out = run(&["run", "--preset", "FAST", "task"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        "error: unknown execution setting \"FAST\" (want balanced or delivery; legacy: economy, full)\n"
    );
}

/// The deprecated `--profile` alias prints its warning to stderr before the
/// unmigrated body; the value still reaches the route.
#[test]
fn run_profile_deprecation_warns_to_stderr() {
    let out = run(&["run", "--profile", "balanced", "task"]);
    assert!(
        out.stderr
            .starts_with(&format!("{PROFILE_DEPRECATED_WARNING}\n")),
        "{}",
        out.stderr
    );
    match &out.route {
        Route::Run { flags, .. } => assert_eq!(flags.value("profile"), Some("balanced")),
        other => panic!("expected Run route, got {other:?}"),
    }
}

/// `--copy` without `--resume` or `--continue` is a usage error on the run
/// path, exit 2 (cli.go:316).
#[test]
fn run_copy_without_session_refused_exit_two() {
    let out = run(&["run", "--copy", "task"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("error: {COPY_REQUIRES_RESUME}\n"));
}

/// `--continue` satisfies the copy guard: a fresh continued start does not copy.
#[test]
fn run_copy_with_continue_passes_guard() {
    let out = run(&["run", "--copy", "--continue", "task"]);
    match &out.route {
        Route::Run { flags, .. } => {
            assert!(flags.bool("copy"));
            assert!(flags.bool("continue"));
        }
        other => panic!("expected Run route, got {other:?}"),
    }
}

/// `--copy --resume <id>` also satisfies the guard.
#[test]
fn run_copy_with_resume_passes_guard() {
    let out = run(&["run", "--copy", "--resume", "abc", "task"]);
    match &out.route {
        Route::Run { flags, .. } => assert_eq!(flags.value("resume"), Some("abc")),
        other => panic!("expected Run route, got {other:?}"),
    }
}

/// The TUI copy guard reports through the resume resolver and exits 1, not 2
/// (tui_command.go:223-227); the non-terminal guard has not fired yet because
/// the copy check precedes it.
#[test]
fn tui_copy_without_session_refused_exit_one() {
    let owned: Vec<String> = ["tui", "--copy"].iter().map(|s| s.to_string()).collect();
    let out = reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe());
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, format!("error: {COPY_REQUIRES_RESUME}\n"));
}
