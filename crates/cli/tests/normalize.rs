//! Argument normalization: CLI-011..CLI-014 and the `--inline` boundary.

use reasonix_cli::flags::normalize_command;
use reasonix_cli::{BuildInfo, CliEnv, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn normalize(argv: &[&str]) -> (String, Vec<String>) {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    normalize_command(&owned)
}

fn run(argv: &[&str]) -> reasonix_cli::CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// CLI-011: `--acp` short-circuits to the `acp` command before any other
/// leading-flag routing.
#[test]
fn acp_shortcut_routes_to_acp() {
    let (cmd, _) = normalize(&["--acp"]);
    assert_eq!(cmd, "acp");
    let out = run(&["--acp"]);
    assert_eq!(out.route, Route::Acp);
}

/// CLI-012: leading flags travel into `run` only when both parsers read the
/// block identically; agreeing cases reroute, disagreeing ones stay the TUI's.
#[test]
fn leading_flags_into_run_when_parsers_agree() {
    let agreed: [(&[&str], &[&str]); 3] = [
        (
            &["--permission-mode", "auto", "run", "hi"],
            &["run", "--permission-mode", "auto", "hi"],
        ),
        (&["-y", "run", "hi"], &["run", "-y", "hi"]),
        (
            &["--yolo", "--model", "m", "run", "--max-steps", "3", "hi"],
            &["run", "--yolo", "--model", "m", "--max-steps", "3", "hi"],
        ),
    ];
    for (argv, want) in agreed {
        let (cmd, args) = normalize(argv);
        assert_eq!(cmd, "run", "{argv:?}");
        assert_eq!(args, want, "{argv:?}");
    }

    let kept: [&[&str]; 5] = [
        &["--model", "m", "run the tests"],
        &["--model", "m", "--", "run", "hi"],
        &["--resume", "run", "hi"],
        &["--model", "m", "--inline", "run", "hi"],
        &["--model", "m", "--metrics", "x", "run", "hi"],
    ];
    for argv in kept {
        let (cmd, args) = normalize(argv);
        assert_eq!(cmd, "", "{argv:?} stays the TUI's");
        assert_eq!(args, argv, "{argv:?}");
    }
}

/// CLI-012: the rerouted line reaches the run route with the merged flags.
#[test]
fn agreed_lead_dispatches_to_run() {
    let out = run(&["--model", "m", "run", "implement the TODOs"]);
    match out.route {
        Route::Run { flags, .. } => {
            assert_eq!(flags.value("model"), Some("m"));
            assert_eq!(flags.positionals, vec!["implement the TODOs".to_owned()]);
        }
        other => panic!("expected Run route, got {other:?}"),
    }
}

/// CLI-013: a leading `-p`/`--print` routes to `run --print`, dropping the
/// first print token so it is not duplicated; a `-p` after the verb stays run's
/// own.
#[test]
fn leading_print_routes_to_run_print() {
    let cases: [(&[&str], &[&str]); 4] = [
        (&["-p", "task"], &["run", "--print", "task"]),
        (&["--print", "task"], &["run", "--print", "task"]),
        (
            &["--model", "x", "-p", "task"],
            &["run", "--print", "--model", "x", "task"],
        ),
        (&["--yolo", "-p", "hi"], &["run", "--print", "--yolo", "hi"]),
    ];
    for (argv, want) in cases {
        let (cmd, args) = normalize(argv);
        assert_eq!(cmd, "run", "{argv:?}");
        assert_eq!(args, want, "{argv:?}");
    }

    // A -p after the verb belongs to run itself; the leading rule does not fire.
    let (cmd, args) = normalize(&["--yolo", "run", "-p", "hi"]);
    assert_eq!(cmd, "run");
    assert_eq!(args, vec!["run", "--yolo", "-p", "hi"]);
}

/// CLI-013: a `-p` behind `--` is a prompt token, not leading print intent.
#[test]
fn print_after_terminator_is_not_leading() {
    let (cmd, args) = normalize(&["--", "-p"]);
    assert_ne!(cmd, "run");
    assert_eq!(args, vec!["--", "-p"]);
}

/// CLI-014: a first token that is a default interactive flag empties the command
/// and enters the bare invocation path; `--inline` and `--auto` do not qualify.
#[test]
fn first_token_interactive_flag_bare_tui() {
    let interactive: Vec<&[&str]> = vec![
        &["--model", "x"],
        &["-c"],
        &["--permission-mode", "ask"],
        &["--permission-mode=ask"],
        &["--resume", "abc"],
        &["--agent-composition", "builtin:default"],
    ];
    for argv in interactive {
        let (cmd, _) = normalize(argv);
        assert_eq!(cmd, "", "{argv:?}");
    }

    // `--inline` is TUI-only: bare it falls through to the unknown command.
    let out = run(&["--inline"]);
    assert_eq!(
        out.route,
        Route::Unknown {
            command: "--inline".to_owned()
        }
    );
    assert_eq!(out.code, 2);

    // `--auto`/`-y` are run approval flags, not bare interactive flags.
    let out = run(&["--auto"]);
    assert!(matches!(out.route, Route::Unknown { .. }));
}
