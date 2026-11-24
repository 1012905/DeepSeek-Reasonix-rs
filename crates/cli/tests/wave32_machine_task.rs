//! `task` top-level matrix, monitor/tmux sub-matrices, and the task machine
//! interface: CLI-032.

use reasonix_cli::text::{
    TASK_MONITOR_UNKNOWN_SUBCOMMAND, TASK_MONITOR_USAGE, TASK_TMUX_UNKNOWN_SUBCOMMAND,
    TASK_TMUX_USAGE, TASK_UNKNOWN_SUBCOMMAND, TASK_USAGE,
};
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// Bare `task` prints the task usage to stderr, exit 2.
#[test]
fn task_bare_usage_exit_two() {
    let out = run(&["task"]);
    assert_eq!(out.stderr, format!("{TASK_USAGE}\n"));
    assert_eq!(out.code, 2);
}

/// An unknown top-level task verb names itself in the diagnostic, exit 2.
#[test]
fn task_unknown_subcommand_exit_two() {
    let out = run(&["task", "frob"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        format!("{}\n", TASK_UNKNOWN_SUBCOMMAND.replace("%s", "frob"))
    );
}

/// `task monitor` requires a subcommand; an unknown one names itself.
#[test]
fn task_monitor_submatrix_guards() {
    let bare = run(&["task", "monitor"]);
    assert_eq!(bare.code, 2);
    assert_eq!(bare.stderr, format!("{TASK_MONITOR_USAGE}\n"));

    let unknown = run(&["task", "monitor", "frob"]);
    assert_eq!(unknown.code, 2);
    assert_eq!(
        unknown.stderr,
        format!(
            "{}\n",
            TASK_MONITOR_UNKNOWN_SUBCOMMAND.replace("%s", "frob")
        )
    );
}

/// `task tmux` requires a subcommand; an unknown one names itself.
#[test]
fn task_tmux_submatrix_guards() {
    let bare = run(&["task", "tmux"]);
    assert_eq!(bare.code, 2);
    assert_eq!(bare.stderr, format!("{TASK_TMUX_USAGE}\n"));

    let unknown = run(&["task", "tmux", "frob"]);
    assert_eq!(unknown.code, 2);
    assert_eq!(
        unknown.stderr,
        format!("{}\n", TASK_TMUX_UNKNOWN_SUBCOMMAND.replace("%s", "frob"))
    );
}

/// `task machine-list --json` renders the empty task collection.
#[test]
fn task_machine_list_json_empty_envelope() {
    let out = run(&["task", "machine-list", "--json"]);
    assert_eq!(out.code, 0);
    assert_eq!(
        out.stdout,
        "{\"schema_version\":1,\"command\":\"task.list\",\"tasks\":[]}"
    );
}

/// `task machine-show` requires a task target; with one it renders the null
/// single-task anchor.
#[test]
fn task_machine_show_target_guard() {
    let missing = run(&["task", "machine-show", "--json"]);
    assert_eq!(missing.code, 2);
    assert!(missing.stdout.contains("\"code\":\"invalid_argument\""));
    assert!(missing.stdout.contains("a task identifier is required"));

    let one = run(&["task", "machine-show", "--json", "task-7"]);
    assert_eq!(one.code, 0);
    assert_eq!(
        one.stdout,
        "{\"schema_version\":1,\"command\":\"task.show\",\"task\":null}"
    );
}

/// Known store-backed top-level verbs route through to the unmigrated anchor.
#[test]
fn task_known_top_level_routes_through() {
    for verb in [
        "list",
        "show",
        "status",
        "events",
        "stop",
        "cancel",
        "requeue",
        "open-session",
    ] {
        let out = run(&["task", verb]);
        let route = &out.route;
        assert!(
            matches!(route, Route::Verb { command, verb: v, .. } if command == "task" && v == verb),
            "{verb}: {out:?}"
        );
    }
}
