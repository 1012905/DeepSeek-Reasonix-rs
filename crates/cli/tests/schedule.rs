//! Hidden `schedule` verbs and the non-human exit 77: CLI-016.

use reasonix_cli::text::{AGENT_CHILD_ENV, SCHEDULE_USAGE};
use reasonix_cli::{BuildInfo, CliEnv, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str], env: &CliEnv) -> reasonix_cli::CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), env)
}

/// No argument and an unknown verb both print the schedule usage on stderr and
/// exit 2.
#[test]
fn schedule_empty_and_unknown_verb_usage_two() {
    let empty = run(&["schedule"], &CliEnv::pipe());
    assert!(empty.stdout.is_empty());
    assert_eq!(empty.stderr, format!("{SCHEDULE_USAGE}\n"));
    assert_eq!(empty.code, 2);

    let unknown = run(&["schedule", "frobnicate"], &CliEnv::pipe());
    assert_eq!(unknown.stderr, format!("{SCHEDULE_USAGE}\n"));
    assert_eq!(unknown.code, 2);
}

/// `resume-all`, `resume`, `renew`, `rm`, and `add` refuse a non-human caller
/// with exit 77 and the terminal-confirmation message.
#[test]
fn schedule_human_verbs_exit_77_non_tty() {
    for verb in ["resume-all", "resume", "renew", "rm", "add"] {
        let argv: &[&str] = match verb {
            "resume-all" | "add" => &["schedule", verb],
            _ => &["schedule", verb, "session-id"],
        };
        let out = run(argv, &CliEnv::pipe());
        assert_eq!(out.code, 77, "{verb}");
        assert_eq!(
            out.stderr,
            format!("schedule {verb} needs a person at a terminal to confirm it\n"),
            "{verb}"
        );
        assert!(matches!(out.route, Route::Schedule { .. }));
    }
}

/// An agent-tool child is refused even on a terminal, with the agent-child
/// marker message.
#[test]
fn schedule_agent_child_exit_77() {
    let mut env = CliEnv::tty();
    env.agent_child = true;
    let out = run(&["schedule", "add"], &env);
    assert_eq!(out.code, 77);
    assert_eq!(
        out.stderr,
        format!(
            "schedule add is refused: this command was started by an agent tool ({AGENT_CHILD_ENV} is set)\n"
        )
    );
}

/// `pause` and `pause-all` do not require a human; they route on instead of
/// hitting exit 77.
#[test]
fn schedule_pause_verbs_do_not_require_human() {
    let pause = run(&["schedule", "pause", "session-id"], &CliEnv::pipe());
    assert_ne!(pause.code, 77);
    assert!(matches!(pause.route, Route::Schedule { verb } if verb == "pause"));

    let pause_all = run(&["schedule", "pause-all"], &CliEnv::pipe());
    assert_ne!(pause_all.code, 77);
    assert!(matches!(pause_all.route, Route::Schedule { verb } if verb == "pause-all"));
}

/// One-id verbs take exactly one argument; a missing id is a usage error (2)
/// before the human guard runs.
#[test]
fn schedule_one_id_verb_requires_single_argument() {
    let out = run(&["schedule", "resume"], &CliEnv::tty());
    assert_eq!(out.stderr, format!("{SCHEDULE_USAGE}\n"));
    assert_eq!(out.code, 2);
}

/// Store-backed verbs route through once the human guard passes.
#[test]
fn schedule_list_routes_through_human_check() {
    let out = run(&["schedule", "list"], &CliEnv::tty());
    assert!(matches!(out.route, Route::Schedule { verb } if verb == "list"));
}

/// `schedule exec` is the supervisor child entry, routed separately.
#[test]
fn schedule_exec_routes_to_exec() {
    let out = run(&["schedule", "exec", "tick"], &CliEnv::pipe());
    assert!(matches!(out.route, Route::Schedule { verb } if verb == "exec"));
}
