//! Account argument guard, `remote` read-only/agent-child matrix, and `trust`
//! parser: CLI-029, CLI-030, CLI-037. The device-flow service, dial/probe
//! backend, and approval store are unmigrated anchors.

use reasonix_cli::text::{REMOTE_REMOVED_STUB, REMOTE_USAGE};
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str], env: &CliEnv) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), env)
}

// --- account ----------------------------------------------------------------

/// `login`/`whoami`/`logout` reject any extra argument, exit 2.
#[test]
fn account_rejects_unexpected_argument() {
    for cmd in ["login", "whoami", "logout"] {
        let out = run(&[cmd, "extra"], &CliEnv::pipe());
        assert_eq!(out.code, 2, "{cmd}");
        assert_eq!(
            out.stderr,
            format!("{cmd}: unexpected argument \"extra\"\n"),
            "{cmd}"
        );
    }
}

// --- remote ------------------------------------------------------------------

/// Bare `remote` prints its usage to stdout, exit 2.
#[test]
fn remote_bare_usage_stdout_exit_two() {
    let out = run(&["remote"], &CliEnv::pipe());
    assert_eq!(out.stdout, format!("{REMOTE_USAGE}\n"));
    assert!(out.stderr.is_empty());
    assert_eq!(out.code, 2);
}

/// `remote help` prints the usage to stdout, exit 0.
#[test]
fn remote_help_usage_stdout_exit_zero() {
    let out = run(&["remote", "help"], &CliEnv::pipe());
    assert_eq!(out.stdout, format!("{REMOTE_USAGE}\n"));
    assert_eq!(out.code, 0);
}

/// An unknown subcommand prints usage to stdout and the diagnostic to stderr,
/// exit 2.
#[test]
fn remote_unknown_subcommand_exit_two() {
    let out = run(&["remote", "frobnicate"], &CliEnv::pipe());
    assert_eq!(out.stdout, format!("{REMOTE_USAGE}\n"));
    assert_eq!(out.stderr, "unknown remote subcommand \"frobnicate\"\n\n");
    assert_eq!(out.code, 2);
}

/// A non-readonly remote verb started by an agent child is refused, exit 77.
#[test]
fn remote_agent_child_refused_exit_seventy_seven() {
    let mut env = CliEnv::pipe();
    env.agent_child = true;
    let out = run(&["remote", "add", "host"], &env);
    assert_eq!(out.code, 77);
    assert!(
        out.stderr.starts_with(
            "reasonix remote add is refused: this command was started by an agent tool"
        )
    );
}

/// Read-only verbs stay allowed for an agent child.
#[test]
fn remote_agent_child_allows_readonly() {
    let mut env = CliEnv::pipe();
    env.agent_child = true;
    for argv in [["remote", "list"], ["remote", "ls"]] {
        let out = run(&argv, &env);
        assert_ne!(out.code, 77, "{argv:?}");
    }
}

/// `wsl add` mutates the machine book and is refused for an agent child;
/// `wsl list`/`wsl test` are read-only and allowed.
#[test]
fn remote_wsl_readonly_split_for_agent_child() {
    let mut env = CliEnv::pipe();
    env.agent_child = true;
    let add = run(&["remote", "wsl", "add", "name", "distro"], &env);
    assert_eq!(add.code, 77);
    let list = run(&["remote", "wsl", "list"], &env);
    assert_ne!(list.code, 77);
    let test = run(&["remote", "wsl", "test", "distro"], &env);
    assert_ne!(test.code, 77);
}

/// The removed Workbench stubs report exit 1 with the migration pointer.
#[test]
fn remote_removed_stubs_exit_one() {
    for stub in [
        "attach-workspace",
        "runtime-workbench",
        "workbench-build-id",
    ] {
        let out = run(&["remote", stub], &CliEnv::pipe());
        assert_eq!(out.code, 1, "{stub}");
        assert!(out.stderr.contains("Remote Workbench 已移除"), "{stub}");
        assert!(
            out.stderr
                .contains(&REMOTE_REMOVED_STUB.replace("%s", stub)),
            "{stub}"
        );
    }
}

// --- trust -------------------------------------------------------------------

/// A bare/unknown trust argument writes the parser error to stdout, exit 2.
#[test]
fn trust_unknown_argument_stdout_exit_two() {
    let out = run(&["trust", "nope"], &CliEnv::pipe());
    assert_eq!(out.code, 2);
    assert!(out.stdout.starts_with("usage: reasonix trust"));
    assert!(out.stdout.contains("unknown argument \"nope\""));
    assert!(out.stderr.is_empty());
}

/// `--yes` and `--revoke` are mutually exclusive.
#[test]
fn trust_yes_and_revoke_conflict() {
    let out = run(&["trust", "--yes", "--revoke"], &CliEnv::pipe());
    assert_eq!(out.code, 2);
    assert!(out.stdout.contains("choose one"));
}

/// `--dir` requires a value.
#[test]
fn trust_dir_requires_value() {
    let out = run(&["trust", "--dir"], &CliEnv::pipe());
    assert_eq!(out.code, 2);
    assert!(out.stdout.contains("--dir"));
}

/// A clean trust argv reaches the approval anchor.
#[test]
fn trust_clean_argv_routes_through() {
    let out = run(&["trust", "--yes", "--allow-providers"], &CliEnv::pipe());
    assert!(matches!(out.route, Route::Verb { command, .. } if command == "trust"));
}
