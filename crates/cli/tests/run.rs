//! `run` flag set and decidable exit-code branches: CLI-019.

use reasonix_cli::text::{
    AUTO_PERMISSION_CONFLICT, AUTO_YOLO_CONFLICT, EVENTS_JSONL_CONFLICT, PLAN_REQUIRES_INTERACTIVE,
    USAGE_RUN_HINT, YOLO_PERMISSION_CONFLICT,
};
use reasonix_cli::{BuildInfo, CliEnv, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> reasonix_cli::CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// The full run flag set parses into the run route; positionals survive as the
/// prompt. Approval aliases are exercised in the conflict and clean-route tests.
#[test]
fn run_flag_set_parses_known_flags() {
    let out = run(&[
        "run",
        "--model",
        "m",
        "--profile",
        "balanced",
        "--preset",
        "delivery",
        "--agent-composition",
        "builtin:default",
        "--max-steps",
        "7",
        "--show-thinking",
        "--debug",
        "--metrics",
        "summary.json",
        "--trajectory",
        "trail.jsonl",
        "--ablate",
        "none",
        "--fold-index",
        "0",
        "--dir",
        "/tmp/ws",
        "--continue",
        "--resume",
        "session-id",
        "--takeover",
        "--copy",
        "--effort",
        "high",
        "--permission-mode",
        "ask",
        "--print",
        "--events-jsonl",
        "--add-dir",
        "/tmp/other",
        "--allowed-tools",
        "Edit",
        "--allowedTools",
        "Bash(git *)",
        "task",
    ]);
    match out.route {
        Route::Run { flags, .. } => {
            assert_eq!(flags.value("model"), Some("m"));
            assert_eq!(flags.value("preset"), Some("delivery"));
            assert_eq!(flags.value("max-steps"), Some("7"));
            assert!(flags.bool("show-thinking"));
            assert!(flags.bool("continue"));
            assert_eq!(flags.value("resume"), Some("session-id"));
            assert!(flags.bool("takeover"));
            assert!(flags.bool("copy"));
            assert!(flags.bool("print"));
            assert!(flags.bool("events-jsonl"));
            assert_eq!(flags.positionals, vec!["task".to_owned()]);
        }
        other => panic!("expected Run route, got {other:?}: {}", out.stderr),
    }
}

/// A clean run with approval flags reaches the run route with parsed values.
#[test]
fn run_clean_argv_routes_with_parsed_flags() {
    let out = run(&["run", "--model", "m", "-y", "implement the TODOs"]);
    match out.route {
        Route::Run { flags, .. } => {
            assert_eq!(flags.value("model"), Some("m"));
            assert!(flags.bool("auto"));
            assert_eq!(flags.positionals, vec!["implement the TODOs".to_owned()]);
        }
        other => panic!("expected Run route, got {other:?}"),
    }
}

/// Approval flag conflicts each map to exit 2 with the reference message.
#[test]
fn run_approval_conflicts_exit_two() {
    let cases: Vec<(&[&str], &str)> = vec![
        (&["run", "-y", "--yolo", "hi"], AUTO_YOLO_CONFLICT),
        (
            &["run", "-y", "--permission-mode", "auto", "hi"],
            AUTO_PERMISSION_CONFLICT,
        ),
        (
            &["run", "--yolo", "--permission-mode", "ask", "hi"],
            YOLO_PERMISSION_CONFLICT,
        ),
    ];
    for (argv, msg) in cases {
        let out = run(argv);
        assert_eq!(out.code, 2, "{argv:?}");
        assert_eq!(out.stderr, format!("error: {msg}\n"), "{argv:?}");
    }
}

/// An unknown `--output-format` value is a usage error.
#[test]
fn run_rejects_unknown_output_format() {
    let out = run(&["run", "--output-format", "xml", "hi"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        "error: unknown output format \"xml\" (want text, json, or stream-json)\n"
    );
}

/// `--events-jsonl` cannot combine with `--output-format`.
#[test]
fn run_events_jsonl_conflicts_with_output_format() {
    let out = run(&["run", "--events-jsonl", "--output-format", "json", "hi"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("error: {EVENTS_JSONL_CONFLICT}\n"));
}

/// `--permission-mode plan` is refused outside an interactive session.
#[test]
fn run_rejects_plan_mode() {
    let out = run(&["run", "--permission-mode", "plan", "hi"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("error: {PLAN_REQUIRES_INTERACTIVE}\n"));
}

/// An empty prompt with empty piped stdin prints the run usage hint, exit 2,
/// without the `error:` prefix.
#[test]
fn run_empty_prompt_usage_hint() {
    let out = run(&["run"]);
    assert!(out.stdout.is_empty());
    assert_eq!(out.stderr, format!("{USAGE_RUN_HINT}\n"));
    assert_eq!(out.code, 2);
}

/// An unknown run flag fails parsing with the generic flag error, exit 2.
#[test]
fn run_unknown_flag_exit_two() {
    let out = run(&["run", "--no-such-flag", "hi"]);
    assert_eq!(out.code, 2);
    assert!(
        out.stderr
            .starts_with("error: unknown flag: --no-such-flag")
    );
}
